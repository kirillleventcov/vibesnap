use crate::error::Result;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

/// Exclusive writer lock on `.vibe/lock`. Every write (an event, a restore, gc)
/// holds it for its whole read-modify-write sequence, so events get their
/// sequence numbers in commit order. Readers never take it. Released on drop.
pub struct WriteLock {
    _file: File,
}

fn open(vibe: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(vibe.join("lock"))?)
}

pub fn lock(vibe: &Path) -> Result<WriteLock> {
    let file = open(vibe)?;
    file.lock()?;
    Ok(WriteLock { _file: file })
}

/// Tries to take the lock for at most `wait`. Hooks use this so they never stall
/// the agent behind a long restore or gc.
pub fn try_lock_for(vibe: &Path, wait: Duration) -> Result<Option<WriteLock>> {
    let file = open(vibe)?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(WriteLock { _file: file })),
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
}
