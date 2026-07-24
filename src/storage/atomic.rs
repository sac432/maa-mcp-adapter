use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use uuid::Uuid;

use super::StorageError;

pub(crate) enum WriteMode {
    Create,
    Replace,
}

pub(crate) fn write_atomically(
    destination: &Path,
    bytes: &[u8],
    mode: WriteMode,
) -> Result<(), StorageError> {
    let parent = destination.parent().ok_or_else(|| StorageError::Io {
        path: destination.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination has no parent directory",
        ),
    })?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("plan");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| StorageError::Io {
                path: temporary.clone(),
                source,
            })?;
        file.write_all(bytes).map_err(|source| StorageError::Io {
            path: temporary.clone(),
            source,
        })?;
        file.flush().map_err(|source| StorageError::Io {
            path: temporary.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| StorageError::Io {
            path: temporary.clone(),
            source,
        })?;
        drop(file);

        match mode {
            WriteMode::Create => {
                fs::rename(&temporary, destination).map_err(|source| StorageError::Io {
                    path: destination.to_path_buf(),
                    source,
                })
            }
            WriteMode::Replace => replace_existing(&temporary, destination),
        }
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_existing(temporary: &Path, destination: &Path) -> Result<(), StorageError> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let temporary_wide = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: Both paths are NUL-terminated UTF-16 buffers that remain alive for the call.
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            temporary_wide.as_ptr(),
            ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            ptr::null(),
            ptr::null(),
        )
    };
    if replaced == 0 {
        Err(StorageError::Io {
            path: destination.to_path_buf(),
            source: std::io::Error::last_os_error(),
        })
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_existing(temporary: &Path, destination: &Path) -> Result<(), StorageError> {
    fs::rename(temporary, destination).map_err(|source| StorageError::Io {
        path: destination.to_path_buf(),
        source,
    })
}
