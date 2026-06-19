# NMSMod - 无人深空模组管理器

一个适用于 macOS 的 No Man's Sky 模组管理器。

## 安装 / 更新程序

```bash
curl -sSL "https://ghfast.top/https://github.com/Tigercrl/NMSMod/releases/latest/download/install.sh" | bash
```

## 使用

### 注入游戏以启用模组

注入后启动游戏即可畅玩模组～

```bash
nmsmod inject
```

### 安装模组

将模组文件夹放入 `~/nmsmod/mods` 目录下。

### 移除游戏注入以禁用模组

```bash
nmsmod inject --remove
```

### 打包 / 解包 HGPAK 文件

```bash
nmsmod pak pack <input_dir> <output_file> <windows/mac/linux>
nmsmod pak unpack <input_file> <output_dir> <windows/mac/linux>
```

### 序列化 MXML 文件 / 反序列化 MBIN 文件

```bash
nmsmod mbin serialize <input_file...> <output_dir>
nmsmod mbin deserialize <input_file...> <output_dir>
```

## 卸载程序

```bash
curl -sSL "https://ghfast.top/https://github.com/Tigercrl/NMSMod/releases/latest/download/uninstall.sh" | bash
```

## 已知问题

### 启动后闪退

1. 移除游戏注入
2. 重新启动原版游戏后退出
3. 重新注入游戏
