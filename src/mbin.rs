use crate::constants::{MBIN_COMPILER_BIN, MBIN_COMPILER_FILE};
use once_cell::sync::Lazy;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;
use thiserror::Error;

/// MBINCompiler 调用与处理相关错误
#[derive(Error, Debug)]
pub enum MbinError {
    #[error("初始化 MBINCompiler 失败: {0}")]
    InitError(String),

    #[error("执行 MBINCompiler 失败: {0}")]
    ExecError(String),
}

/// 全局持有的编译器临时运行目录与执行文件路径，在程序退出时自动销毁
static COMPILER: Lazy<Result<(TempDir, PathBuf), MbinError>> = Lazy::new(|| {
    let temp_dir = TempDir::new().map_err(|e| MbinError::InitError(format!("{e}")))?;
    let exe_path = temp_dir.path().join(MBIN_COMPILER_FILE);

    fs::write(&exe_path, MBIN_COMPILER_BIN).map_err(|e| MbinError::InitError(format!("{e}")))?;

    fs::set_permissions(&exe_path, fs::Permissions::from_mode(0o755))
        .map_err(|e| MbinError::InitError(format!("{e}")))?;

    Ok((temp_dir, exe_path))
});

/// 获取编译器的静态执行路径
fn get_compiler_path() -> Result<&'static PathBuf, MbinError> {
    COMPILER
        .as_ref()
        .map(|(_, p)| p)
        .map_err(|e| MbinError::InitError(e.to_string()))
}

/// MXML 转 MBIN（序列化操作）
pub fn serialize_mxml(paths: &[&str], output_dir: &str) -> Result<(), MbinError> {
    if paths.is_empty() {
        return Ok(());
    }
    let exe = get_compiler_path()?;
    let output = Command::new(&exe)
        .arg("convert")
        .arg("-Q")
        .arg("-y")
        .arg("-f")
        .arg("-iMXML")
        .arg("-oMBIN")
        .arg(format!("--output-dir={}", output_dir))
        .args(paths)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| MbinError::ExecError(format!("{e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(MbinError::ExecError(stderr));
    }
    Ok(())
}

/// MBIN 转 MXML（反序列化操作）
pub fn deserialize_mbin(paths: &[&str], output_dir: &str) -> Result<(), MbinError> {
    if paths.is_empty() {
        return Ok(());
    }
    let exe = get_compiler_path()?;
    let output = Command::new(&exe)
        .arg("convert")
        .arg("-Q")
        .arg("-y")
        .arg("-f")
        .arg("-iMBIN")
        .arg("-oMXML")
        .arg(format!("--output-dir={}", output_dir))
        .args(paths)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| MbinError::ExecError(format!("{e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(MbinError::ExecError(stderr));
    }
    Ok(())
}
