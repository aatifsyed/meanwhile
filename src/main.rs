use std::{fs, io, iter, path::PathBuf, process, thread, time::Duration};

use anyhow::Context;
use clap::Parser;
use tap::Pipe;
use tracing::{debug, error, info};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Meanwhile {
    cmd: String,
    args: Vec<String>,
    stdout_suffix: Option<String>,
    stderr_suffix: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct MeanwhileFile {
    meanwhile: Vec<Meanwhile>,
}

#[derive(Debug, clap::Parser)]
#[command(about, version)]
struct Args {
    /// Path to config file specifying background tasks
    #[arg(short, long, default_value = "./meanwhile.toml")]
    meanwhile_file: PathBuf,
    /// How long to sleep in seconds after starting the background tasks
    #[arg(short, long, default_value_t = 1.0)]
    sleep_after_spawn: f64,
    /// The prefix to give to all output files
    #[arg(short, long)]
    name: Option<String>,
    /// Interactively edit the shared name for files before saving
    #[arg(short, long)]
    interactive_name: bool,
    /// If present, save the `stdout` of `CMD`
    #[arg(short = 'o', long)]
    stdout_suffix: Option<String>,
    /// If present, save the `stderr` of `CMD`
    #[arg(short = 'e', long)]
    stderr_suffix: Option<String>,
    /// Where to save the output files
    #[arg(short = 'd', long, default_value = ".")]
    outdir: PathBuf,
    /// The command to run
    #[arg(last(true), num_args(1..), required(true))]
    cmd: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .pretty()
        .with_file(false)
        .without_time()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let args = Args::parse();
    debug!(?args);

    let meanwhile_file = fs::read_to_string(&args.meanwhile_file)
        .with_context(|| {
            format!(
                "couldn't open meanwhile-file {}",
                args.meanwhile_file.display()
            )
        })?
        .pipe_as_ref(toml::from_str::<MeanwhileFile>)
        .with_context(|| {
            format!(
                "couldn't deserialize meanwhile-file {} as toml",
                args.meanwhile_file.display()
            )
        })?;
    debug!(?meanwhile_file);

    let children = meanwhile_file
        .meanwhile
        .into_iter()
        .map(
            |Meanwhile {
                 cmd,
                 args,
                 stdout_suffix,
                 stderr_suffix,
             }| {
                info!(%cmd, ?args, "spawning background task");
                process::Command::new(&cmd)
                    .args(args)
                    .stdout(process::Stdio::piped())
                    .stderr(process::Stdio::piped())
                    .stdin(process::Stdio::null())
                    .spawn()
                    .map(|child| {
                        debug!(pid = child.id());
                        (child, stdout_suffix, stderr_suffix)
                    })
                    .with_context(|| format!("couldn't spawn background task with command {}", cmd))
            },
        )
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut cmd_and_args = args.cmd;
    let cmd = cmd_and_args.remove(0);
    let cmd_args = cmd_and_args;

    info!(
        seconds = args.sleep_after_spawn,
        "waiting before running main task"
    );
    thread::sleep(Duration::from_secs_f64(args.sleep_after_spawn));
    info!(%cmd, ?cmd_args, "running main task");
    // todo: handle ctrl+c, and still collect outputs
    let output = process::Command::new(&cmd)
        .args(cmd_args)
        .output()
        // OS will clean up our children
        .with_context(|| format!("couldn't execute main task with command {cmd}"))?;

    info!(?output, "main task exited");

    for (child, _, _) in children.iter() {
        debug!(pid = child.id(), "interrupting child");
        if let Err(errno) = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.id() as _),
            nix::sys::signal::SIGINT,
        ) {
            error!(%errno, pid = child.id(), "couldn't interrupt child");
        }
    }

    // Allow children to handle sigints
    thread::sleep(Duration::from_millis(100));

    let outputs = children
        .into_iter()
        .filter_map(|(mut child, stdout_suffix, stderr_suffix)| {
            let pid = child.id();
            match child.kill() {
                Ok(_) => {
                    debug!(pid, "killed child");
                    wait_or_log(child, stdout_suffix, stderr_suffix, pid)
                }
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                    debug!(pid, "child already exited");
                    wait_or_log(child, stdout_suffix, stderr_suffix, pid)
                }
                Err(error) => {
                    error!(
                        ?error,
                        pid, "unable to kill child, output will not be collected"
                    );
                    None
                }
            }
        });

    fs::create_dir_all(&args.outdir)
        .with_context(|| format!("couldn't create outdir {}", args.outdir.display()))?;
    let name = match args.interactive_name {
        true => dialoguer::Input::new()
            .with_prompt("select name:")
            .with_initial_text(args.name.unwrap_or_default())
            .interact_text()
            .context("couldn't select name")?,
        false => args.name.unwrap_or_default(),
    };

    for (output, stdout_suffix, stderr_suffix) in
        iter::once((output, args.stdout_suffix, args.stderr_suffix)).chain(outputs)
    {
        debug!(?output, ?stdout_suffix, ?stderr_suffix);
        write_or_log(stdout_suffix, &args.outdir, &name, &output.stdout);
        write_or_log(stderr_suffix, &args.outdir, &name, &output.stderr);
    }

    Ok(())
}

fn write_or_log(maybe_suffix: Option<String>, outdir: &PathBuf, name: &String, output: &Vec<u8>) {
    if let Some(suffix) = maybe_suffix {
        let file = outdir.join(format!("{name}{suffix}"));
        if let Err(error) = fs::write(&file, output) {
            error!(?error, file = %file.display(), "coudn't write output");
        }
    }
}

fn wait_or_log(
    child: process::Child,
    stdout_suffix: Option<String>,
    stderr_suffix: Option<String>,
    pid: u32,
) -> Option<(process::Output, Option<String>, Option<String>)> {
    match child.wait_with_output() {
        Ok(output) => {
            info!(?output, "captured background task output");
            Some((output, stdout_suffix, stderr_suffix))
        }
        Err(error) => {
            error!(
                ?error,
                pid, "unable to wait on child, output will not be collected"
            );
            None
        }
    }
}
