//! Filesystem capability with permission enforcement. All plugin-originated
//! calls are scoped by the permission evaluator (path traversal and symlink
//! escapes are contained by canonicalization before any operation runs).

use std::path::{Path, PathBuf};

use crate::{CallerCtx, KResult, Kernel, KernelError};

const READ_TEXT_MAX: u64 = 4 * 1024 * 1024;

pub fn check(kernel: &Kernel, caller: &CallerCtx, write: bool, path: &Path) -> KResult<PathBuf> {
    let Some(plugin_id) = &caller.plugin_id else {
        return Ok(path.to_path_buf());
    };
    let ws_root = kernel
        .current_workspace()
        .map(|ws| ws.workspace.root().to_path_buf());
    kernel
        .permissions
        .check_fs(plugin_id, write, path, ws_root.as_deref())
        .map_err(|e| KernelError::Permission(e.to_string()))
}

pub fn read_dir_entries(path: &Path) -> KResult<serde_json::Value> {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        entries.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "isDir": meta.is_dir(),
            "isFile": meta.is_file(),
            "isSymlink": entry.file_type()?.is_symlink(),
            "size": if meta.is_file() { meta.len() } else { 0 },
            "modified": meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64),
        }));
    }
    entries.sort_by(|a, b| {
        let dir_a = !a["isDir"].as_bool().unwrap_or(false);
        let dir_b = !b["isDir"].as_bool().unwrap_or(false);
        dir_a.cmp(&dir_b).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
        })
    });
    Ok(serde_json::json!({ "entries": entries, "path": path.display().to_string() }))
}

pub fn read_text(path: &Path, max: u64) -> KResult<serde_json::Value> {
    let meta = std::fs::metadata(path)
        .map_err(|e| KernelError::Message(format!("cannot stat `{}`: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(KernelError::Message(format!(
            "`{}` is not a file",
            path.display()
        )));
    }
    let limit = max.min(READ_TEXT_MAX);
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    let mut take = (&mut file).take(limit + 1);
    take.read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > limit;
    if truncated {
        bytes.truncate(limit as usize);
    }
    let mut text = String::from_utf8_lossy(&bytes).to_string();
    // Don't split mid-codepoint.
    if truncated {
        while !text.is_empty()
            && text
                .chars()
                .last()
                .map(|c| c as u32 > 0x10FFFF)
                .unwrap_or(false)
        {
            text.pop();
        }
    }
    Ok(serde_json::json!({
        "content": text,
        "truncated": truncated,
        "size": meta.len(),
    }))
}

pub fn read_base64(path: &Path) -> KResult<serde_json::Value> {
    let data = std::fs::read(path)
        .map_err(|e| KernelError::Message(format!("cannot read `{}`: {e}", path.display())))?;
    Ok(serde_json::json!({
        "content": b64_encode(&data),
        "size": data.len(),
    }))
}

pub fn write_bytes(path: &Path, content: &[u8], create_dirs: bool) -> KResult<()> {
    if create_dirs {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                KernelError::Message(format!("cannot create `{}`: {e}", parent.display()))
            })?;
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            return Err(KernelError::Message(format!(
                "parent directory `{}` does not exist",
                parent.display()
            )));
        }
    }
    std::fs::write(path, content)
        .map_err(|e| KernelError::Message(format!("cannot write `{}`: {e}", path.display())))
}

pub fn delete(path: &Path, recursive: bool, workspace_root: Option<&Path>) -> KResult<()> {
    if let Some(root) = workspace_root {
        if path == root {
            return Err(KernelError::Message(
                "refusing to delete the workspace root".into(),
            ));
        }
    }
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| KernelError::Message(format!("cannot stat `{}`: {e}", path.display())))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        if recursive {
            std::fs::remove_dir_all(path).map_err(|e| {
                KernelError::Message(format!("cannot remove `{}`: {e}", path.display()))
            })
        } else {
            std::fs::remove_dir(path).map_err(|e| {
                KernelError::Message(format!("`{}` is a directory: {e}", path.display()))
            })
        }
    } else {
        std::fs::remove_file(path)
            .map_err(|e| KernelError::Message(format!("cannot remove `{}`: {e}", path.display())))
    }
}

pub fn copy_path(from: &Path, to: &Path) -> KResult<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            KernelError::Message(format!("cannot create `{}`: {e}", parent.display()))
        })?;
    }
    let meta = std::fs::symlink_metadata(from)
        .map_err(|e| KernelError::Message(format!("cannot stat `{}`: {e}", from.display())))?;
    if meta.is_dir() {
        wz_plugin_runtime::copy_dir(from, to)
            .map_err(|e| KernelError::Message(format!("copy failed: {e}")))
    } else {
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| KernelError::Message(format!("copy failed: {e}")))
    }
}

pub fn stat_entry(path: &Path) -> KResult<serde_json::Value> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| KernelError::Message(format!("cannot stat `{}`: {e}", path.display())))?;
    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "isDir": meta.is_dir(),
        "isFile": meta.is_file(),
        "isSymlink": meta.file_type().is_symlink(),
        "size": meta.len(),
        "created": meta.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
        "modified": meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64),
    }))
}

/// Standard base64 (RFC 4648) without external dependencies.
pub fn b64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn b64_decode(s: &str) -> KResult<Vec<u8>> {
    fn val(c: u8) -> KResult<u32> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(KernelError::Message("invalid base64 input".into())),
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            return Err(KernelError::Message("invalid base64 length".into()));
        }
        let pad = 4 - chunk.len();
        let mut n: u32 = 0;
        for (i, c) in chunk.iter().enumerate() {
            let v = if *c == b'=' && i >= 2 { 0 } else { val(*c)? };
            n |= v << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 && chunk[2] != b'=' {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 && chunk[3] != b'=' {
            out.push(n as u8);
        }
        let _ = pad;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for input in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[0u8, 255, 10, 7],
        ] {
            let enc = b64_encode(input);
            let dec = b64_decode(&enc).unwrap();
            assert_eq!(dec, input, "roundtrip failed for {input:?}");
        }
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(b64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_rejects_garbage() {
        assert!(b64_decode("!!!").is_err());
    }
}
