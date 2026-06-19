#!/bin/bash

# 配置路径
BINARY_PATH="/usr/local/bin/nmsmod"
DATA_DIR="$HOME/nmsmod"

echo "➡️ 开始卸载 无人深空模组管理器 (nmsmod)..."

echo "🔒 请输入密码以获取管理员权限："
sudo

if [ -f "$BINARY_PATH" ]; then
    # 1. 移除游戏注入
    echo "🔧 正在移除游戏注入..."
    $BINARY_PATH inject --remove

    # 2. 删除可执行文件
    echo "📦 正在删除可执行文件: $BINARY_PATH"
    sudo rm -f "$BINARY_PATH"
    if [ $? -eq 0 ]; then
        echo "✅ 成功删除可执行文件。"
    else
        echo "❌ 删除可执行文件失败，请检查权限。"
        exit 1
    fi
else
    echo "ℹ️ 未在 $BINARY_PATH 找到可执行文件，跳过该步骤。"
fi

# 3. 询问是否删除数据和配置文件
if [ -d "$DATA_DIR" ]; then
    echo ""
    # 提示用户输入，默认回车为不删除 (N)
    read -p "❓ 是否删除所有模组及配置目录 '$DATA_DIR'？(y/N): " choice
    case "$choice" in
        [yY][eE][sS]|[yY])
            echo "🗑️ 正在删除数据目录: $DATA_DIR..."
            rm -rf "$DATA_DIR"
            if [ $? -eq 0 ]; then
                echo "✅ 成功删除数据目录。"
            else
                echo "❌ 删除数据目录失败，请手动检查权限。"
            fi
            ;;
        *)
            echo "ℹ️ 已保留数据及配置目录 '$DATA_DIR'，以便日后重新安装时使用。"
            ;;
    esac
else
    echo "ℹ️ 未找到数据目录 '$DATA_DIR'，跳过该步骤。"
fi

echo ""
echo "✅ 卸载完成！"
