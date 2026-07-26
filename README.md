# quarkdrive-webdav
夸克网盘 WebDAV 服务

[![Docker Image](https://img.shields.io/badge/version-latest-blue)](https://ghcr.io/chenqimiao/quarkdrive-webdav)
[![Crates.io](https://img.shields.io/crates/v/quarkdrive-webdav.svg)](https://crates.io/crates/quarkdrive-webdav)

 
 ## 核心特性
- 🐳 Docker 容器化部署 - 快速启动，零环境依赖，无需繁琐配置即可完成部署
- 📦 二进制包 + 命令行启动 - 支持直接下载二进制包，通过命令行快速启动，部署方式灵活多样
- 🌱 极致轻量级运行 - 仅需约 10MB 内存占用，低资源消耗特性使其可流畅运行在低配置环境中
- 🔄 NAS 与云盘双向同步 - 支持 NAS 与云盘间文件基于WebDAV协议的备份、下载、上传操作
- 🎬 云盘影音无缝播放 - 完美适配  [Infuse](https://firecore.com/infuse)、[nPlayer](https://nplayer.com) 等支持 WebDAV 协议的客户端 App直接播放云盘内容
- 🚀 流式上传 - 边接收边推送到云端，不在本地落盘。上传数十 GB 的文件也不会产生本地缓存，客户端并发数可随意设置
- 📁 群晖 Cloud Sync 兼容 - 已针对群晖 Cloud Sync（含客户端加密）实测调优，大文件同步不再出现缓存堆积或"配额已达上限"中断



如果项目对你有帮助，欢迎 Star 或者赞助我，以支持本项目的继续开发

## 支付码

<p align="center">
  <img src="https://github.com/chenqimiao/chenqimiao/raw/main/pic/alipay.JPG" alt="alipay" width="400" height="400" style="margin-right: 40px;"/>
  <img src="https://github.com/chenqimiao/chenqimiao/raw/main/pic/wechat_pay.JPG" alt="wechat_pay" width="400" height="400"/>
</p>

## 💖 鸣谢捐赠

衷心感谢以下朋友的支持，正是你们的鼓励让本项目得以持续迭代 🙏

| 日期 | 渠道 | 捐赠者 | 金额 |
| :---: | :---: | :---: | :---: |
| 2026-06-06 | WeChat | J\*o | ¥100.00 |
| 2026-03-26 | WeChat | M\*u | ¥50.00 |
| 2026-03-25 | WeChat | \*途 | ¥10.00 |
| 2025-08-06 | WeChat | \*平 | ¥18.50 |
| 2025-05-04 | WeChat | L\*s | ¥100.00 |
| 2025-01-07 | WeChat | \*良 | ¥25.00 |
| **合计** |  | **5 位** | **¥303.50** |


> **Note**
>
> 本项目作者没有上传需求, 所以上传实现较为简单，测试场景不能全部覆盖，后续会慢慢优化

## 二进制安装

### 从 GitHub Releases 下载

可以从 [GitHub Releases](https://github.com/chenqimiao/quarkdrive-webdav/releases) 页面下载预先构建的二进制包，支持 Linux、macOS、Windows 多平台。

### 通过 Cargo 安装

如果已安装 [Rust](https://www.rust-lang.org/tools/install) 工具链，可以直接通过 Cargo 安装：

```bash
cargo install quarkdrive-webdav
```

## 命令行启动

```bash
quarkdrive-webdav --quark-cookie '你的cookie' -U '用户名' -W '密码' -p 8080
```


## Docker 

### docker run
```bash
docker run -d --name=quarkdrive-webdav --restart=unless-stopped -p 8080:8080 \
  -e QUARK_COOKIE='your quark cookie' \
  -e WEBDAV_AUTH_USER=admin \
  -e WEBDAV_AUTH_PASSWORD=admin \
  ghcr.io/chenqimiao/quarkdrive-webdav:latest
```

### docker compose

```yaml
version: '3.8'
services:
  quarkdrive-webdav:
    image: ghcr.io/chenqimiao/quarkdrive-webdav:latest
    container_name: quarkdrive-webdav
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      - QUARK_COOKIE=your quark cookie
      - WEBDAV_AUTH_USER=admin
      - WEBDAV_AUTH_PASSWORD=admin
```

其中，`QUARK_COOKIE` 环境变量为你的夸克云盘 `cookie`，`WEBDAV_AUTH_USER`
和 `WEBDAV_AUTH_PASSWORD` 为连接 WebDAV 服务的用户名和密码。



启动后，用webdav客户端或者浏览器连接http://nas地址:8080 即可


## 上传行为与相关配置

客户端在 PUT 请求里带了 `Content-Length` 时，上传会**边收边传**直接推到云端，不在本地落盘。读取客户端的速度被推送云端的速度拖住，靠 TCP 背压让客户端自然降速，因此不会出现"本地缓存越堆越大"的情况。

只有不提供 `Content-Length` 的客户端才会回落到"先完整落盘、再上传"的旧路径。下面的参数几乎只对这条回落路径生效。

| 环境变量 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `UPLOAD_TEMP_DIR` | `/tmp` | 回落路径的暂存目录。**很多宿主上 `/tmp` 是 tmpfs（即内存）**，几个并发的大文件足以把整台机器拖垮，Docker 镜像里已改为 `/var/cache/quarkdrive`。 |
| `UPLOAD_WAIT_TIMEOUT` | `280` | 回落路径专用：等待上传完成多少秒后提前给客户端返回成功，上传转入后台继续。设为 `0` 则一直等到真正传完。用于避免客户端自身超时。 |

### 关于内存

流式上传每个文件在内存里最多保留一个分片，分片大小由云端决定（实测为 4–20MB）。因此上传期间的内存约为 `并发数 × 分片大小`，例如 10 个大文件并发约 200MB。空闲时仍然只占约 10MB。

### 关于并发数

流式上传不占用本地磁盘，客户端并发数不会导致缓存堆积，可以按需设置。但注意两点：

- **总吞吐由上行带宽决定**，提高并发只是把同一条管道切成更多份，不会更快
- **并发越高，单个大文件的传输时间越长**。流式上传的分片一旦失败无法续传，需要整个文件重来，因此传输时间越长、重来的代价越大

小文件为主的场景适合高并发（能填满 API 往返的空隙），大文件为主建议 2–4。

### 群晖 Cloud Sync

已针对群晖 Cloud Sync 实测（含客户端加密、双向同步）。加密只改变文件内容与大小，不改变文件名，`Content-Length` 始终存在，因此走流式路径。

需要注意的是，**Cloud Sync 的客户端加密使同一文件每次加密的结果都不同**，云端秒传因而无法命中，每次重新同步都是全量重传。这是加密本身的性质，流式上传使其只消耗带宽，不再消耗本地磁盘。

## 🚨 免责声明

本项目仅供学习和研究目的，不得用于任何商业活动。用户在使用本项目时应遵守所在地区的法律法规，对于违法使用所导致的后果，本项目及作者不承担任何责任。
本项目可能存在未知的缺陷和风险（包括但不限于设备损坏和账号封禁等），使用者应自行承担使用本项目所产生的所有风险及责任。
作者不保证本项目的准确性、完整性、及时性、可靠性，也不承担任何因使用本项目而产生的任何损失或损害责任。
使用本项目即表示您已阅读并同意本免责声明的全部内容。
