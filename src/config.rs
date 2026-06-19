use crate::MAIN_DIR;
use crate::constants::CONFIG_FILE;
use crate::game::{detect_game_dir, is_game_dir_valid};
use colored::*;
use serde::{Deserialize, Serialize};
use std::env::home_dir;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

/// 全局配置结构体
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub game_dir: PathBuf,
    pub pack_batch_chunk_count: usize,
    pub mbin_concurrency: usize,
}

/// 配置相关错误
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("无法读取配置文件: {0}")]
    ReadError(#[from] std::io::Error),

    #[error("配置文件解析失败: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("配置文件字段错误: {0}")]
    ValidationError(String),
}

impl Config {
    /// 加载并验证配置文件
    pub fn load() -> Result<Self, ConfigError> {
        let content = fs::read_to_string(home_dir().unwrap().join(MAIN_DIR).join(CONFIG_FILE))?;
        let config: Config = serde_json::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// 持久化保存配置文件
    pub fn save(&self) -> Result<(), ConfigError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(home_dir().unwrap().join(MAIN_DIR).join(CONFIG_FILE), json)?;
        Ok(())
    }

    /// 校验配置字段合法性
    fn validate(&self) -> Result<(), ConfigError> {
        if let Err(e) = is_game_dir_valid(&self.game_dir) {
            return Err(ConfigError::ValidationError(e.to_string()));
        }
        if self.pack_batch_chunk_count == 0 {
            return Err(ConfigError::ValidationError(
                "pack_batch_chunk_count 必须大于 0".to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for Config {
    /// 生成默认配置（自动检测游戏目录）
    fn default() -> Self {
        let game_dir = detect_game_dir();
        if let Err(e) = &game_dir {
            eprintln!("⚠️自动检测游戏目录失败: {}", e.to_string().yellow());
        }
        Config {
            game_dir: game_dir.unwrap_or_else(|_| PathBuf::new()),
            pack_batch_chunk_count: 256,
            mbin_concurrency: 8,
        }
    }
}
