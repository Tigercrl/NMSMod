#!/bin/bash

# 仓库与文件配置
BINARY_NAME="nmsmod"
DEST_DIR="/usr/local/bin"
FINAL_NAME="nmsmod"
DOWNLOAD_URL="https://ghfast.top/https://github.com/Tigercrl/NMSMod/releases/latest/download/nmsmod"

echo "⬇️ 正在下载: $BINARY_NAME"
echo "🔗 链接: $DOWNLOAD_URL"

# 下载文件
curl -L -o "$FINAL_NAME" "$DOWNLOAD_URL"

# 检查下载是否成功
if [ $? -ne 0 ]; then
    echo "❌ 下载失败！"
    rm -f "$FINAL_NAME"
    exit 1
fi

echo "📦 正在安装到 $DEST_DIR/$FINAL_NAME"

echo "🔒 请输入密码以获取管理员权限："

# 移动文件
sudo mv "$FINAL_NAME" "$DEST_DIR/$FINAL_NAME"
sudo chmod +x "$DEST_DIR/$FINAL_NAME"

# 验证安装
if [ $? -eq 0 ]; then
    echo "✅ 安装成功！"
else
    echo "❌ 安装过程中出现错误，请检查权限。"
    exit 1
fi
