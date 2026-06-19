use std::io;
use thiserror::Error;

/// HGPAK 相关操作的错误枚举
#[derive(Debug, Error)]
pub enum Error {
    /// 标准 IO 错误
    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),

    /// HGPAK 文件格式或版本无效
    #[error("无效的 HGPAK 文件: {0}")]
    InvalidFile(String),

    /// 数据压缩失败
    #[error("压缩错误: {0}")]
    Compression(String),

    /// 数据解压失败
    #[error("解压错误: {0}")]
    Decompression(String),

    /// 归档内未找到指定文件
    #[error("文件未找到: {0}")]
    FileNotFound(String),

    /// 归档文件损坏或数据不一致
    #[error("归档文件损坏: {0}")]
    CorruptedArchive(String),
}

/// 全局 Result 类型别名
pub type Result<T> = std::result::Result<T, Error>;
