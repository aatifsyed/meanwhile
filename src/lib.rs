use std::process::Stdio;

use flo_stream::{MessagePublisher, ToPublisherSink};
use futures::{Stream, StreamExt, TryStreamExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tracing::error;

fn stdio(child: &mut Child) -> (ChildStdin, ChildStdout, ChildStderr) {
    (
        child.stdin.take().unwrap(),
        child.stdout.take().unwrap(),
        child.stderr.take().unwrap(),
    )
}

pub fn foo() {
    let mut child = Command::new("echo")
        .args(["hello", "hello", "world"])
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .expect("couldn't spawn task");
    let (_stdin, stdout, stderr) = stdio(&mut child);
    let publisher = flo_stream::Publisher::new(10);
    tokio_util::io::ReaderStream::new(stdout)
        .map_err(|error| {
            error!(?error, "couldn't read from stdout");
            ()
        })
        .forward(publisher.republish().to_sink());
    // publisher.subscribe();
}

#[cfg(test)]
mod tests {
    use color_eyre::eyre::bail;

    use super::*;

    #[tracing::instrument()]
    fn bar(i: i32) -> color_eyre::Result<()> {
        match i {
            0 => Ok(()),
            _ => bail!("nonzero"),
        }
    }

    #[test]
    fn test() -> color_eyre::Result<()> {
        color_eyre::install()?;
        bar(1)
    }
}
