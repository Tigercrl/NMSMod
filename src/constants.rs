/// 全局常量定义配置

/// 工具主目录名称
pub const MAIN_DIR: &str = "nmsmod";

/// 缓存与模组存储目录
pub const CACHE_DIR: &str = "cache";
pub const MODS_DIR: &str = "mods";

/// 配置文件与关键组件定义
pub const CONFIG_FILE: &str = "config.json";
pub const HASH_FILE: &str = "hash";
pub const MBIN_COMPILER_FILE: &str = "MBINCompiler";

/// 游戏内部路径与应用文件结构
pub const PAK_PATH: &str = "Contents/Resources/GAMEDATA/MACOSBANKS";
pub const MOD_PAK: &str = "NMSMOD.pak";
pub const PLIST_FILE: &str = "Contents/Info.plist";
pub const STARTUP_EXEC_PATH: &str = "Contents/MacOS/";
pub const EXEC_FILE: &str = "No Man's Sky";
pub const LAUNCHER_FILE: &str = "nmsmod_launcher";

/// 嵌入的二进制程序数据
pub const LAUNCHER_BIN: &[u8] = include_bytes!("../bin/nmsmod_launcher");
pub const MBIN_COMPILER_BIN: &[u8] = include_bytes!("../bin/MBINCompiler");