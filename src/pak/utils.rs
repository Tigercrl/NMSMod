use crate::pak::compression::Compressor;
use md5::{Digest, Md5};
use rayon::prelude::*;
use std::io::Write;

/// 计算数据存储所需的固定分块数
pub fn determine_bins(num_bytes: u64, bin_size: u64) -> u64 {
    (num_bytes + bin_size - 1) / bin_size
}

/// 计算满足 16 字节对齐所需的补零填充字节数
pub fn padding(x: u64) -> u64 {
    (0x10 - (x & 0xF)) & 0xF
}

/// 规范化路径：将反斜杠替换为正斜杠并统一转为小写
pub fn normalise_path(path: &str) -> String {
    path.replace("\\", "/").to_lowercase()
}

/// 提取规范化后路径的 MD5 签名
pub fn hash_path(path: &str) -> [u8; 16] {
    let mut hasher = Md5::new();
    hasher.update(normalise_path(path).as_bytes());
    hasher.finalize().into()
}

/// 支持并发多块打包处理的高速串行化编码写入器
pub struct RepackChunkWriter<'a, W: Write> {
    compressor: &'a Compressor,
    compress: bool,
    writer: &'a mut W,
    chunk_size: usize,
    batch_limit: usize,
    current_chunk: Vec<u8>,
    batch: Vec<Vec<u8>>,
    pub compressed_block_sizes: Vec<u64>,
}

/// 供结构填充复用的对齐零缓冲区
const ZERO_PAD: [u8; 16] = [0u8; 16];

impl<'a, W: Write> RepackChunkWriter<'a, W> {
    /// 构造新的分块串行写入上下文
    pub fn new(
        compressor: &'a Compressor,
        compress: bool,
        batch_limit: usize,
        writer: &'a mut W,
    ) -> Self {
        let chunk_size = compressor.decompressed_chunk_size;
        Self {
            compressor,
            compress,
            writer,
            chunk_size,
            batch_limit,
            current_chunk: Vec::with_capacity(chunk_size),
            batch: Vec::with_capacity(batch_limit),
            compressed_block_sizes: Vec::new(),
        }
    }

    /// 馈送任意大小的字节流切片，自动切块并在达到上限时自动触发批量并发压缩
    pub fn push_slice(&mut self, mut data: &[u8]) -> crate::pak::error::Result<()> {
        while !data.is_empty() {
            let space = self.chunk_size - self.current_chunk.len();
            let take = space.min(data.len());
            self.current_chunk.extend_from_slice(&data[..take]);
            data = &data[take..];

            if self.current_chunk.len() == self.chunk_size {
                let full =
                    std::mem::replace(&mut self.current_chunk, Vec::with_capacity(self.chunk_size));
                self.batch.push(full);
                if self.batch.len() >= self.batch_limit {
                    self.flush_batch()?;
                }
            }
        }
        Ok(())
    }

    /// 终止写入并强行对齐刷新尾部的残余块，返回已持久化的全量块尺度统计表
    pub fn finish(mut self) -> crate::pak::error::Result<Vec<u64>> {
        if !self.current_chunk.is_empty() {
            self.current_chunk.resize(self.chunk_size, 0u8);
            let full =
                std::mem::replace(&mut self.current_chunk, Vec::with_capacity(self.chunk_size));
            self.batch.push(full);
        }
        self.flush_batch()?;
        self.writer.flush()?;
        Ok(self.compressed_block_sizes)
    }

    /// 并行压缩批处理通道内的暂存块，并强行保证物理对齐后安全刷入流底
    fn flush_batch(&mut self) -> crate::pak::error::Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }

        let chunks = std::mem::take(&mut self.batch);

        if self.compress {
            let encoded_results: Vec<crate::pak::error::Result<(Vec<u8>, u64)>> = chunks
                .into_par_iter()
                .map(|raw| {
                    let compressed = self.compressor.compress(&raw)?;
                    // 若压缩后体积未发生缩减则直接降级存储原始块
                    let encoded = if compressed.len() >= raw.len() {
                        raw
                    } else {
                        compressed
                    };
                    let size = encoded.len() as u64;
                    Ok((encoded, size))
                })
                .collect();

            for item in encoded_results {
                let (block, size) = item?;
                self.writer.write_all(&block)?;
                let pad = padding(size);
                if pad != 0 {
                    self.writer.write_all(&ZERO_PAD[..pad as usize])?;
                }
                self.compressed_block_sizes.push(size);
            }
        } else {
            // 未压缩模式：直通输出
            for block in chunks {
                self.writer.write_all(&block)?;
            }
        }

        Ok(())
    }
}
