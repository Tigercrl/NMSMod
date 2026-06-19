//! # HGPAK 文件格式说明
//!
//! HGPAK 是一种将多个文件打包为单个归档文件的格式，支持可选的压缩（Zstd 或 LZ4）。
//! 文件结构如下：
//!
//! 1. **头部 (Header)**：固定 0x30 字节
//!    - Magic: b"HGPAK\0\0\0" (8 字节)
//!    - Version: u64 (小端)
//!    - FileCount: u64 (小端)    — 文件总数（包含清单文件）
//!    - ChunkCount: u64 (小端)   — 压缩块数量（未压缩时为 0）
//!    - IsCompressed: u8 (1 压缩, 0 未压缩) + 7 字节填充
//!    - DataOffset: u64 (小端)   — 数据区起始偏移量
//!
//! 2. **文件信息表 (FileInfo Table)**：每个文件 0x20 字节
//!    - FileHash: [u8; 16]       — 文件路径的 MD5 哈希
//!    - StartOffset: u64         — 文件数据起始偏移（绝对）
//!    - DecompressedSize: u64    — 解压后大小
//!
//! 3. **压缩块大小表 (Chunk Size Table)**：仅当 `IsCompressed == true` 时存在
//!    - 共 ChunkCount 个 u64 值，每个表示该压缩块压缩后的大小（字节）
//!    - 块大小表之后可能会补充填充字节，使数据区起始地址对齐到 0x10 边界。
//!
//! 4. **数据区 (Data Section)**
//!    - 若未压缩：连续存储每个文件的原始数据，每个文件大小向上取整到 16 字节对齐。
//!    - 若压缩：数据被分为固定大小的块（Zstd: 0x10000，LZ4: 0x20000），每个块独立压缩，
//!      并填充至 16 字节对齐。文件可能跨越多个块，需要根据偏移和大小计算所在块范围。
//!
//! 5. **清单文件 (Manifest)**：第一个文件（索引 0）固定为清单文件，
//!    内容为 UTF-8 文本，每行一个文件路径（相对于清单所在目录），使用 `\r\n` 换行。
//!    后续文件的路径顺序与清单中的顺序一一对应。

use crate::pak::compression::Compressor;
use crate::pak::compression::Platform;
use crate::pak::error::{Error, Result};
use crate::pak::utils::{RepackChunkWriter, determine_bins, hash_path, normalise_path, padding};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use memmap2::MmapOptions;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// 包内清单文件的固定相对路径
const REPACK_MANIFEST_PATH: &str = ".hgpak_manifest";
/// 当前支持的 HGPAK 文件版本
const HGPAK_VERSION: u64 = 2;

/// 原始文件元数据条目
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// 路径的 MD5 哈希值
    pub file_hash: [u8; 16],
    /// 文件数据在包中的绝对起始偏移
    pub start_offset: u64,
    /// 文件解压后的大小
    pub decompressed_size: u64,
}

/// 解析后的文件路由与分块定位元数据
#[derive(Debug, Clone)]
pub struct PackedFile {
    /// 数据在数据区内的相对偏移（未压缩模式下即为文件绝对偏移）
    pub offset: u64,
    /// 文件解压后的大小
    pub size: u64,
    /// 包内原始路径
    pub path: String,
    /// 占用的压缩块范围 (起始块索引, 结束块索引)
    pub in_chunks: (u64, u64),
    /// 在起始块内的相对偏移
    pub first_chunk_offset: u64,
    /// 在结束块内的结束位置（0 表示刚好到块末尾）
    pub last_chunk_offset_end: u64,
}

/// HGPAK 全局元数据文件头
#[derive(Debug, Default)]
pub struct HGPakHeader {
    pub version: u64,
    pub file_count: u64,
    pub chunk_count: u64,
    pub is_compressed: bool,
    pub data_offset: u64,
}

impl HGPakHeader {
    /// 从数据流中读取并验证文件头
    pub fn read<R: Read + Seek>(file: &mut R) -> Result<Self> {
        file.seek(SeekFrom::Start(0))?;
        let mut magic = [0u8; 5];
        file.read_exact(&mut magic)?;
        if &magic != b"HGPAK" {
            return Err(Error::InvalidFile("不是 HGPAK 文件".to_string()));
        }
        file.seek(SeekFrom::Start(8))?; // 跳过 Magic 填充区域

        let version = file.read_u64::<LittleEndian>()?;
        if version != HGPAK_VERSION {
            return Err(Error::InvalidFile(format!(
                "不支持的 HGPAK 文件版本: {}",
                version
            )));
        }

        let file_count = file.read_u64::<LittleEndian>()?;
        let chunk_count = file.read_u64::<LittleEndian>()?;
        let is_compressed = file.read_u8()? != 0;
        file.seek(SeekFrom::Current(7))?; // 跳过标志位填充
        let data_offset = file.read_u64::<LittleEndian>()?;

        Ok(Self {
            version,
            file_count,
            chunk_count,
            is_compressed,
            data_offset,
        })
    }

    /// 将文件头串行化写入数据流
    pub fn write<W: Write>(&self, file: &mut W) -> Result<()> {
        file.write_all(b"HGPAK\0\0\0")?;
        file.write_u64::<LittleEndian>(self.version)?;
        file.write_u64::<LittleEndian>(self.file_count)?;
        file.write_u64::<LittleEndian>(self.chunk_count)?;
        file.write_u8(if self.is_compressed { 1 } else { 0 })?;
        file.write_all(&[0u8; 7])?;
        file.write_u64::<LittleEndian>(self.data_offset)?;
        Ok(())
    }
}

/// HGPAK 归档操作句柄，提供解包与检索功能
pub struct HGPAKFile {
    pub filepath: PathBuf,
    pub compressor: Compressor,
    pub batch_chunks: usize,
    pub header: HGPakHeader,
    pub file_info: Vec<FileInfo>,
    pub chunk_sizes: Vec<u64>,
    pub chunk_offsets: Vec<u64>,
    pub files: HashMap<String, PackedFile>,
}

impl HGPAKFile {
    /// 打开现有的 HGPAK 文件并初始化内部索引表
    pub fn open<P: AsRef<Path>>(
        filepath: P,
        platform: Platform,
        batch_chunks: Option<usize>,
    ) -> Result<Self> {
        let mut file = File::open(&filepath)?;
        let header = HGPakHeader::read(&mut file)?;
        let compressor = Compressor::new(platform.get_compression());
        let batch_chunks = batch_chunks.unwrap_or(256);

        // 读取元数据信息表
        let mut file_info = Vec::with_capacity(header.file_count as usize);
        for _ in 0..header.file_count {
            let mut hash = [0u8; 16];
            file.read_exact(&mut hash)?;
            let start_offset = file.read_u64::<LittleEndian>()?;
            let decompressed_size = file.read_u64::<LittleEndian>()?;
            file_info.push(FileInfo {
                file_hash: hash,
                start_offset,
                decompressed_size,
            });
        }

        let mut chunk_sizes = Vec::with_capacity(header.chunk_count as usize);
        let mut chunk_offsets = Vec::with_capacity(header.chunk_count as usize);

        // 压缩模式下加载压缩块元数据并计算物理偏移
        if header.is_compressed {
            file.seek(SeekFrom::Start(0x30 + header.file_count * 0x20))?;
            let mut curr_offset = header.data_offset;
            for _ in 0..header.chunk_count {
                let size = file.read_u64::<LittleEndian>()?;
                chunk_sizes.push(size);
                chunk_offsets.push(curr_offset);
                curr_offset += (size + 15) & !15; // 块对齐到 16 字节边界
            }
        }

        let mut pak = Self {
            filepath: filepath.as_ref().to_path_buf(),
            compressor,
            batch_chunks,
            header,
            file_info,
            chunk_sizes,
            chunk_offsets,
            files: HashMap::new(),
        };

        pak.parse_filenames(&mut file)?;
        Ok(pak)
    }

    /// 解析内置清单文件以建立路径映射索引
    fn parse_filenames(&mut self, file: &mut File) -> Result<()> {
        if self.file_info.is_empty() {
            return Ok(());
        }

        // 清单文件固定保存在索引 0 位置
        let manifest_size = self.file_info[0].decompressed_size;
        let mut manifest_data = Vec::with_capacity(manifest_size as usize);

        if !self.header.is_compressed {
            // 未压缩模式：直读数据区
            file.seek(SeekFrom::Start(self.header.data_offset))?;
            let mut buf = vec![0u8; manifest_size as usize];
            file.read_exact(&mut buf)?;
            manifest_data = buf;
        } else {
            // 压缩模式：跨块读取并解压清单数据
            let chunks_needed = determine_bins(
                manifest_size,
                self.compressor.decompressed_chunk_size as u64,
            );
            for i in 0..chunks_needed {
                let idx = i as usize;
                if idx >= self.chunk_offsets.len() || idx >= self.chunk_sizes.len() {
                    return Err(Error::CorruptedArchive(
                        "清单引用的块索引超出范围".to_string(),
                    ));
                }
                file.seek(SeekFrom::Start(self.chunk_offsets[idx]))?;
                let mut buf = vec![0u8; self.chunk_sizes[idx] as usize];
                file.read_exact(&mut buf)?;
                let decomp = self.compressor.decompress(&buf)?;
                manifest_data.extend_from_slice(&decomp);
            }
        }

        manifest_data.truncate(manifest_size as usize);
        let text = String::from_utf8_lossy(&manifest_data);
        let filenames: Vec<&str> = text.split("\r\n").filter(|s| !s.is_empty()).collect();

        // 关联路径名与对应的文件元数据条目
        for (i, filename) in filenames.iter().enumerate() {
            if i + 1 >= self.file_info.len() {
                break;
            }
            let file_info = &self.file_info[i + 1];

            let start_offset = if self.header.is_compressed {
                file_info
                    .start_offset
                    .checked_sub(self.header.data_offset)
                    .ok_or_else(|| {
                        Error::CorruptedArchive(format!("文件 '{}' 的起始偏移无效无效", filename))
                    })?
            } else {
                file_info.start_offset
            };

            if file_info.decompressed_size == 0 {
                self.files.insert(
                    filename.to_string(),
                    PackedFile {
                        offset: start_offset,
                        size: 0,
                        path: filename.to_string(),
                        in_chunks: (0, 0),
                        first_chunk_offset: 0,
                        last_chunk_offset_end: 0,
                    },
                );
                continue;
            }

            let cs = self.compressor.decompressed_chunk_size as u64;
            let start_chunk = start_offset / cs;
            let end_chunk = (start_offset + file_info.decompressed_size - 1) / cs;

            if self.header.is_compressed && end_chunk >= self.chunk_offsets.len() as u64 {
                return Err(Error::CorruptedArchive(format!(
                    "文件 '{}' 的块索引 {} 超出范围",
                    filename, end_chunk
                )));
            }

            self.files.insert(
                filename.to_string(),
                PackedFile {
                    offset: start_offset,
                    size: file_info.decompressed_size,
                    path: filename.to_string(),
                    in_chunks: (start_chunk, end_chunk),
                    first_chunk_offset: start_offset % cs,
                    last_chunk_offset_end: (start_offset + file_info.decompressed_size) % cs,
                },
            );
        }

        Ok(())
    }

    /// 解压单个包内文件到目标路径
    pub fn unpack_single_file(&self, internal_path: &str, out_path: &Path) -> Result<()> {
        let packed_file = self.files.get(internal_path).ok_or_else(|| {
            Error::FileNotFound(format!("在 PAK 中未找到文件: {}", internal_path))
        })?;

        if let Some(p) = out_path.parent() {
            std::fs::create_dir_all(p)?;
        }

        let mut out_file = BufWriter::with_capacity(1 << 20, File::create(out_path)?);

        if packed_file.size == 0 {
            out_file.flush()?;
            return Ok(());
        }

        let source = File::open(&self.filepath)?;
        let mmap = unsafe { MmapOptions::new().map(&source)? };

        if self.header.is_compressed {
            let mut chunk_indices = Vec::new();
            let mut idx = packed_file.in_chunks.0;
            while idx <= packed_file.in_chunks.1 {
                chunk_indices.push(idx);
                idx += 1;
            }

            // 分批并行执行解压任务
            for group in chunk_indices.chunks(self.batch_chunks) {
                let group_indices: Vec<u64> = group.to_vec();

                let raw_chunks: Vec<&[u8]> = group_indices
                    .iter()
                    .map(|&chunk_idx| {
                        let idx = chunk_idx as usize;
                        if idx >= self.chunk_offsets.len() || idx >= self.chunk_sizes.len() {
                            return Err(Error::CorruptedArchive(
                                "解包时块索引超出范围".to_string(),
                            ));
                        }
                        let start = self.chunk_offsets[idx] as usize;
                        let end = start + self.chunk_sizes[idx] as usize;
                        Ok(&mmap[start..end])
                    })
                    .collect::<Result<Vec<_>>>()?;

                let decoded_results: Vec<Result<Vec<u8>>> = raw_chunks
                    .into_par_iter()
                    .map(|chunk| self.compressor.decompress(chunk))
                    .collect();

                // 处理解压后的分块切片并写入目标文件
                for (pos, decoded) in decoded_results.into_iter().enumerate() {
                    let decomp = decoded?;
                    let chunk_idx = group_indices[pos];

                    let mut start = 0usize;
                    let mut end = decomp.len();

                    if chunk_idx == packed_file.in_chunks.0 {
                        start = packed_file.first_chunk_offset as usize;
                    }
                    if chunk_idx == packed_file.in_chunks.1 {
                        end = if packed_file.last_chunk_offset_end == 0 {
                            decomp.len()
                        } else {
                            packed_file.last_chunk_offset_end as usize
                        };
                    }

                    if start <= end && end <= decomp.len() {
                        out_file.write_all(&decomp[start..end])?;
                    }
                }
            }
        } else {
            // 未压缩模式：直接切片映射区域拷贝
            let start = packed_file.offset as usize;
            let end = start + packed_file.size as usize;
            out_file.write_all(&mmap[start..end])?;
        }

        out_file.flush()?;
        Ok(())
    }

    /// 解包归档中的所有文件到指定目录
    pub fn unpack<F>(&self, dest: &Path, mut progress: Option<&mut F>) -> Result<()>
    where
        F: FnMut(&str, usize, usize),
    {
        let total = self.files.len();
        for (idx, file_path) in self.files.keys().enumerate() {
            if let Some(ref mut cb) = progress {
                cb(file_path, idx + 1, total);
            }
            let out_path = dest.join(file_path);
            self.unpack_single_file(file_path, &out_path)?;
        }
        Ok(())
    }

    /// 将一组外部文件或目录重新打包为新的 HGPAK 文件
    pub fn repack<F>(
        files: Vec<(PathBuf, String)>,
        out_path: &Path,
        compress: bool,
        platform: Platform,
        batch_chunks: Option<usize>,
        mut progress: Option<&mut F>,
    ) -> Result<()>
    where
        F: FnMut(&str, usize, usize),
    {
        let mut expanded: Vec<(PathBuf, String)> = Vec::new();

        // 递归展平输入流中的目录节点
        for (real_path, internal_path) in files {
            if real_path.is_dir() {
                for entry in WalkDir::new(&real_path)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .filter(|e| e.file_name() != ".DS_Store")
                {
                    let file_type = entry.file_type();
                    if !file_type.is_file() {
                        continue;
                    }
                    let full_path = entry.path();
                    let relative = full_path
                        .strip_prefix(&real_path)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let base = normalise_path(&internal_path);
                    let inner = if base.is_empty() {
                        relative
                    } else {
                        format!("{}/{}", base, relative)
                    };
                    let norm_inner = normalise_path(&inner);
                    expanded.push((full_path.to_path_buf(), norm_inner));
                }
            } else {
                let norm = normalise_path(&internal_path);
                expanded.push((real_path, norm));
            }
        }

        let mut internal_paths = Vec::with_capacity(expanded.len());
        let mut path_map = HashMap::new();

        for (real_path, internal_path) in expanded {
            let norm = normalise_path(&internal_path);
            if path_map.contains_key(&norm) {
                return Err(Error::InvalidFile(format!("重复的路径: {}", norm)));
            }
            path_map.insert(norm.clone(), real_path);
            internal_paths.push(norm);
        }

        // 构建清单文件字节流
        let manifest_content = internal_paths
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("\r\n")
            + "\r\n";
        let manifest_bytes = manifest_content.as_bytes();
        let manifest_norm = normalise_path(REPACK_MANIFEST_PATH);

        let mut all_internal_paths = Vec::with_capacity(1 + internal_paths.len());
        all_internal_paths.push(manifest_norm.clone());
        all_internal_paths.extend(internal_paths);

        let compressor = Compressor::new(platform.get_compression());
        let chunk_size = compressor.decompressed_chunk_size as u64;

        let mut file_infos = Vec::with_capacity(all_internal_paths.len());
        let mut total_data_size = 0u64;

        // 插入清单文件的描述条目
        let manifest_hash = hash_path(&manifest_norm);
        let manifest_size = manifest_bytes.len() as u64;
        file_infos.push(FileInfo {
            file_hash: manifest_hash,
            start_offset: total_data_size,
            decompressed_size: manifest_size,
        });
        total_data_size += manifest_size;
        if let Some(ref mut cb) = progress {
            cb(".hgpak_manifest", 1, 1 + all_internal_paths.len() - 1);
        }

        // 插入各个普通用户文件的描述条目
        for (idx, path) in all_internal_paths[1..].iter().enumerate() {
            if let Some(ref mut cb) = progress {
                cb(path, idx + 2, 1 + all_internal_paths.len() - 1);
            }

            let real_path = path_map.get(path).ok_or_else(|| {
                Error::FileNotFound(format!("内部路径缺失对应的实际路径: {}", path))
            })?;

            let metadata = std::fs::metadata(real_path)?;
            let size = metadata.len();
            file_infos.push(FileInfo {
                file_hash: hash_path(path),
                start_offset: total_data_size,
                decompressed_size: size,
            });
            total_data_size += size;
        }

        let file_count = file_infos.len() as u64;
        let chunk_count = if compress {
            determine_bins(total_data_size, chunk_size)
        } else {
            0
        };

        // 计算表布局与对齐填充
        let chunk_table_offset = 0x30 + 0x20 * file_count;
        let chunk_table_len = if compress { 0x8 * chunk_count } else { 0 };
        let extra_padding = padding(chunk_table_offset + chunk_table_len);
        let data_offset = chunk_table_offset + chunk_table_len + extra_padding;

        for finfo in &mut file_infos {
            finfo.start_offset += data_offset;
        }

        let header = HGPakHeader {
            version: HGPAK_VERSION,
            file_count,
            chunk_count,
            is_compressed: compress,
            data_offset,
        };

        let mut out_file = BufWriter::with_capacity(1 << 20, File::create(out_path)?);
        header.write(&mut out_file)?;

        for file_info in &file_infos {
            out_file.write_all(&file_info.file_hash)?;
            out_file.write_u64::<LittleEndian>(file_info.start_offset)?;
            out_file.write_u64::<LittleEndian>(file_info.decompressed_size)?;
        }

        // 预留块大小表的写入槽位
        if compress {
            let pad_len = (chunk_table_len + extra_padding) as usize;
            if pad_len > 0 {
                let zeros = vec![0u8; pad_len];
                out_file.write_all(&zeros)?;
            }
        }

        // 驱动分块写入器压缩并串行化全量数据
        let batch_chunks = batch_chunks.unwrap_or(256);
        let compressed_block_sizes = {
            let mut chunk_writer =
                RepackChunkWriter::new(&compressor, compress, batch_chunks, &mut out_file);
            chunk_writer.push_slice(manifest_bytes)?;

            for path in &all_internal_paths[1..] {
                let real_path = path_map.get(path).unwrap();
                let input = File::open(real_path)?;
                let mmap = unsafe { MmapOptions::new().map(&input)? };
                chunk_writer.push_slice(&mmap)?;
            }

            chunk_writer.finish()?
        };

        out_file.flush()?;

        // 回填准确的实际压缩块尺寸映射表
        if compress {
            out_file.seek(SeekFrom::Start(chunk_table_offset))?;
            for size in &compressed_block_sizes {
                out_file.write_u64::<LittleEndian>(*size)?;
            }
            out_file.flush()?;
        }

        Ok(())
    }
}
