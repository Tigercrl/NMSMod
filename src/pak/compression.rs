use crate::pak::error::{Error, Result};
use lz4::block::{CompressionMode, compress as lz4_compress, decompress as lz4_decompress};
use zstd::bulk::{compress as zstd_compress, decompress as zstd_decompress};

/// 构建目标平台
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Platform {
    Windows,
    Mac,
    Linux,
}

impl Platform {
    /// 检索与目标平台关联的默认压缩格式策略
    pub fn get_compression(&self) -> Compression {
        match self {
            Platform::Windows | Platform::Linux => Compression::Zstd,
            Platform::Mac => Compression::Lz4,
        }
    }
}

/// 支持的后端底层压缩编解码器
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Zstd,
    Lz4,
}

/// 统一的压缩数据块处理器
pub struct Compressor {
    pub compression: Compression,
    /// 压缩算法对应的固定逻辑分块大小
    pub decompressed_chunk_size: usize,
}

impl Compressor {
    /// 根据底层算法初始化对应的处理器实例
    pub fn new(compression: Compression) -> Self {
        let decompressed_chunk_size = match compression {
            Compression::Zstd => 0x10000,
            Compression::Lz4 => 0x20000,
        };
        Self {
            compression,
            decompressed_chunk_size,
        }
    }

    /// 执行数据块压缩
    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        match self.compression {
            Compression::Zstd => {
                zstd_compress(data, 14).map_err(|e| Error::Compression(e.to_string()))
            }
            Compression::Lz4 => {
                lz4_compress(data, Some(CompressionMode::HIGHCOMPRESSION(10)), false)
                    .map_err(|e| Error::Compression(e.to_string()))
            }
        }
    }

    /// 执行数据块解压缩
    pub fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        // 大小刚好相等说明存储的是未压缩的原始原始块直通数据
        if data.len() == self.decompressed_chunk_size {
            return Ok(data.to_vec());
        }

        match self.compression {
            Compression::Zstd => zstd_decompress(data, self.decompressed_chunk_size)
                .map_err(|e| Error::Decompression(e.to_string())),
            Compression::Lz4 => lz4_decompress(data, Some(self.decompressed_chunk_size as i32))
                .map_err(|e| Error::Decompression(e.to_string())),
        }
    }
}
