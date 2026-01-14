# Rust Telegram Forwarder 部署与更新手册（Debian 13）

本文档描述：本地编译 → 上传 VPS → systemd 托管 → 日志与排障。

## 1. 环境约定

- 本地编译环境：WSL Debian 13 (trixie)
- VPS 运行环境：Debian 13 (trixie)
- VPS SSH：`<USER>@<VPS_IP>`（默认端口 22）
- 本地项目路径（示例）：`/mnt/d/your/path/rust_tdlib`
- VPS 部署目录（示例）：`/opt/tgbot`
- systemd 服务：`tgbot.service`
- 可执行文件名称：`app`

## 2. 本地编译准备

### 2.1 安装依赖

```bash
sudo apt update
sudo apt install -y \
  build-essential pkg-config \
  clang cmake \
  libsqlite3-dev \
  curl ca-certificates \
  rsync openssh-client
```

### 2.2 安装 Rust 工具链

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
rustc --version
cargo --version
```

### 2.3 本地编译（release）

```bash
cd /mnt/d/your/path/rust_tdlib
cargo build --release
```

如遇到构建异常，可使用干净构建：

```bash
cargo clean
cargo build --release
```

## 3. 上传到 VPS

### 3.1 仅更新二进制（推荐）

```bash
rsync -avz --progress -e "ssh -p 22" \
  target/release/app \
  <USER>@<VPS_IP>:/opt/tgbot/app
```

### 3.2 同步目录（需要同步配置时）

注意源路径末尾的 `/`：

```bash
rsync -avz --progress -e "ssh -p 22" \
  /mnt/d/your/path/rust_tdlib/ \
  <USER>@<VPS_IP>:/opt/tgbot/
```

## 4. VPS 首次部署（systemd 托管）

> 已部署过可跳过此节。

### 4.1 创建目录与权限

```bash
ssh -p 22 <USER>@<VPS_IP>
mkdir -p /opt/tgbot
chmod +x /opt/tgbot/app
chmod 600 /opt/tgbot/.env 2>/dev/null || true
```

### 4.2 创建 systemd 服务文件

编辑：

```bash
sudo nano /etc/systemd/system/tgbot.service
```

内容示例：

```ini
[Unit]
Description=Telegram Bot (tgbot)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/tgbot
ExecStart=/opt/tgbot/app
EnvironmentFile=/opt/tgbot/.env

Restart=on-failure
RestartSec=3
StartLimitIntervalSec=60
StartLimitBurst=10

NoNewPrivileges=true
LimitNOFILE=65535
OOMScoreAdjust=-200

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

### 4.3 启动并设置开机自启

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now tgbot.service
sudo systemctl status tgbot.service --no-pager
```

## 5. 日常更新流程

### 5.1 本地编译 + 上传

```bash
cd /mnt/d/your/path/rust_tdlib
cargo build --release

rsync -avz --progress -e "ssh -p 22" \
  target/release/app \
  <USER>@<VPS_IP>:/opt/tgbot/app
```

### 5.2 VPS 重启并确认

```bash
ssh -p 22 <USER>@<VPS_IP>
chmod +x /opt/tgbot/app
sudo systemctl restart tgbot.service
sudo systemctl status tgbot.service --no-pager
journalctl -u tgbot.service -n 50 --no-pager
```

## 6. 日志与排障

### 6.1 实时日志

```bash
journalctl -u tgbot.service -f
```

### 6.2 查看最近日志

```bash
journalctl -u tgbot.service -n 1000 --no-pager
```

### 6.3 常见问题

- `./app: No such file or directory`：多为 GLIBC 版本或架构不匹配，需保证本地与 VPS 版本一致。
- 服务启动失败：优先检查 `.env` 格式是否符合 systemd `EnvironmentFile` 规范。
- SSH 粘贴乱码（如 `^[[200~`）：建议手敲命令，或执行：

```bash
bind 'set enable-bracketed-paste off'
```

## 7. 限制 journald 日志占用（可选）

```bash
journalctl --disk-usage
sudo mkdir -p /etc/systemd/journald.conf.d
sudo tee /etc/systemd/journald.conf.d/limit.conf <<'EOF'
[Journal]
SystemMaxUse=200M
SystemMaxFileSize=50M
EOF
sudo systemctl restart systemd-journald
```

## 8. 快速命令清单

### 本地（WSL Debian 13）编译 + 上传

```bash
cd /mnt/d/your/path/rust_tdlib
cargo build --release
rsync -avz --progress -e "ssh -p 22" target/release/app <USER>@<VPS_IP>:/opt/tgbot/app
```

### VPS 重启 + 查看日志

```bash
chmod +x /opt/tgbot/app
sudo systemctl restart tgbot.service
sudo systemctl status tgbot.service --no-pager
journalctl -u tgbot.service -n 50 --no-pager
```
