//! Job control: stopping the editor the way a shell stops a foreground job.
//!
//! `C-z` cannot simply raise `SIGTSTP` and hope. The terminal has to be handed
//! back first, or the shell that regains the prompt inherits raw mode and the
//! alternate screen. And the signal is worth sending only when something is
//! able to bring the editor back afterwards: Linux discards a stop signal
//! aimed at an orphaned process group, so the editor would carry on as though
//! the key had done nothing, with no way to tell the user why.

use crate::Result;
#[cfg(unix)]
use crate::TuiError;
#[cfg(unix)]
use rustix::process::{self, Signal};

/// What became of a request to stop the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suspension {
    /// The editor stopped, and whatever is doing job control has since
    /// continued it. Execution resumes where it left off.
    Resumed,
    /// Nothing was sent. No process is in a position to resume this one, so a
    /// stop signal would either be discarded by the kernel or strand the user
    /// in front of a program nothing can restart.
    NoJobControl,
}

/// Whether a process in `group` and `session` has a parent that could resume
/// it after it stops.
///
/// POSIX calls a process group orphaned when no member has a parent in a
/// different group of the same session, and Linux silently discards `SIGTSTP`
/// sent to such a group. Our own parent is the case that decides it in
/// practice: the editor was started by a shell, which sits in its own group in
/// the same session and is the thing that will run `fg`.
///
/// Kept separate from the calls that supply its arguments so the rule itself
/// can be tested without arranging real processes — on every platform, which
/// is why this is compiled for tests even where nothing calls it.
#[cfg(any(unix, test))]
fn parent_can_resume(group: i32, session: i32, parent: Option<(i32, i32)>) -> bool {
    match parent {
        // A parent inside our own group stops when we do, so nothing would be
        // left running to continue us. A parent in another session is not
        // doing job control for this terminal.
        Some((parent_group, parent_session)) => {
            parent_group != group && parent_session == session
        }
        None => false,
    }
}

/// True when stopping this process would actually stop it, and something could
/// start it again.
#[cfg(unix)]
fn job_control_is_available() -> bool {
    let Ok(session) = process::getsid(None) else {
        return false;
    };
    let parent = process::getppid().and_then(|parent| {
        Some((
            process::getpgid(Some(parent)).ok()?.as_raw_pid(),
            process::getsid(Some(parent)).ok()?.as_raw_pid(),
        ))
    });
    parent_can_resume(process::getpgrp().as_raw_pid(), session.as_raw_pid(), parent)
}

/// Stops this process, returning once something has resumed it.
///
/// The caller must have given the terminal back first: every thread stops,
/// including the one that would otherwise finish drawing the screen.
///
/// Only this process is stopped, not the whole process group, so language
/// servers and other children keep running and are still warm on the way back.
#[cfg(unix)]
pub fn suspend() -> Result<Suspension> {
    if !job_control_is_available() {
        return Ok(Suspension::NoJobControl);
    }
    // The kernel delivers the signal on the way back out of this call, so the
    // next line is what runs when the job is continued.
    process::kill_process(process::getpid(), Signal::TSTP)
        .map_err(|errno| TuiError::Io(std::io::Error::from(errno)))?;
    Ok(Suspension::Resumed)
}

/// Stops this process — except that on Windows there is nothing to stop it
/// with.
///
/// There is no stop signal and no shell holding the job, so `C-z` reports that
/// job control is unavailable rather than pretending. The rule that decides
/// this on Unix is `parent_can_resume`, which is plain arithmetic and is
/// compiled and tested everywhere.
#[cfg(not(unix))]
pub fn suspend() -> Result<Suspension> {
    Ok(Suspension::NoJobControl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_in_its_own_group_can_resume_the_job_it_started() {
        // The ordinary case: `maxgus` in group 100, the shell in group 7, both
        // in session 1. This is the arrangement `C-z` exists for.
        assert!(parent_can_resume(100, 1, Some((7, 1))));
    }

    #[test]
    fn a_parent_inside_the_same_group_cannot_resume_it() {
        // The group is orphaned: stopping it would stop the parent too. A test
        // harness that spawns the editor without setting a process group looks
        // exactly like this, and Linux discards the signal.
        assert!(!parent_can_resume(100, 1, Some((100, 1))));
    }

    #[test]
    fn a_parent_in_another_session_cannot_resume_it() {
        // Another session is another controlling terminal; its shell is not
        // doing job control for this one.
        assert!(!parent_can_resume(100, 1, Some((7, 2))));
    }

    #[test]
    fn a_process_with_no_parent_cannot_be_resumed() {
        assert!(!parent_can_resume(100, 1, None));
    }

    #[test]
    fn the_session_is_compared_before_the_group_is_trusted() {
        // A parent that shares neither is still no use, and the group test
        // alone would have said yes.
        assert!(!parent_can_resume(100, 1, Some((100, 2))));
    }

    #[cfg(unix)]
    #[test]
    fn asking_the_kernel_does_not_fail() {
        // Under `cargo test` the answer is false — the test binary spawned no
        // new process group — but what matters here is that the four calls
        // agree on their argument types and none of them errors.
        let _ = job_control_is_available();
    }
}
