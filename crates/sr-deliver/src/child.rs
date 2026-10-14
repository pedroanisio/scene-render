//! Taking the pipes of a spawned tool.

use std::process::Child;

/// What `take` pulls out of `child` (its pipes), or `None` with the child
/// killed and reaped when it was spawned without them, so that a caller
/// that gives up on the child leaves no process behind.
pub(crate) fn take_or_reap<T>(child: &mut Child, take: impl FnOnce(&mut Child) -> Option<T>) -> Option<T> {
    let taken = take(child);
    if taken.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    taken
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn sleeper(stdout: Stdio) -> Child {
        Command::new("sh").args(["-c", "sleep 30"]).stdin(Stdio::null()).stdout(stdout).spawn().unwrap()
    }

    #[test]
    fn a_child_spawned_without_the_pipe_is_killed_and_reaped() {
        let mut child = sleeper(Stdio::null());
        assert!(take_or_reap(&mut child, |c| c.stdout.take()).is_none());
        // reaped: the exit status is already known, and it is not a clean exit
        let status = child.try_wait().unwrap().expect("the child was reaped");
        assert!(!status.success());
    }

    #[test]
    fn a_child_with_the_pipe_hands_it_over_and_keeps_running() {
        let mut child = sleeper(Stdio::piped());
        assert!(take_or_reap(&mut child, |c| c.stdout.take()).is_some());
        assert!(child.try_wait().unwrap().is_none(), "the child must still be running");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn missing_one_of_two_pipes_reaps_the_child() {
        let mut child = sleeper(Stdio::piped());
        assert!(take_or_reap(&mut child, |c| c.stdout.take().zip(c.stderr.take())).is_none());
        assert!(child.try_wait().unwrap().is_some());
    }
}
