# quarkdrive-webdav
夸克网盘 WebDAV 服务

> **本仓库 fork 自 [chenqimiao/quarkdrive-webdav](https://github.com/chenqimiao/quarkdrive-webdav)**，围绕"把上传做对"做了如下改动：
>
> - **上传改为流式**。原实现先把整个文件写进本地暂存目录、收完再上传；局域网接收速度远快于上行带宽，大文件同步必然堆积（实测堆到过 20GB）。现在边收边推，不落盘。
> - **修掉"上传成功却查不到文件"**。上传完成后有两个竞态会让客户端在 PUT 刚成功时收到 404：夸克后端的最终一致性窗口，以及目录缓存"先读后写"期间被失效冲掉、导致旧列表复活。现在会确认文件确实出现在目录列表里，才向客户端返回成功。
> - **目录列表并发合并**。同一目录的并发查询共用一次请求，不再每个请求都向夸克拉一遍全量列表。
> - **实测兼容群晖 Cloud Sync 上传**。含客户端加密的同步任务，从 KB 级字幕到 4.76GB 视频全部通过。
>
> 上游的功能与用法保持不变，下面的文档同样适用。

[![Docker Image](https://img.shields.io/badge/version-latest-blue)](https://github.com/zmzhuai/quarkdrive-webdav)

 
 ## 核心特性
- 🐳 Docker 容器化部署 - 快速启动，零环境依赖，无需繁琐配置即可完成部署
- 📦 二进制包 + 命令行启动 - 支持直接下载二进制包，通过命令行快速启动，部署方式灵活多样
- 🌱 极致轻量级运行 - 仅需约 10MB 内存占用，低资源消耗特性使其可流畅运行在低配置环境中
- 🔄 NAS 与云盘双向同步 - 支持 NAS 与云盘间文件基于WebDAV协议的备份、下载、上传操作
- 🎬 云盘影音无缝播放 - 完美适配  [Infuse](https://firecore.com/infuse)、[nPlayer](https://nplayer.com) 等支持 WebDAV 协议的客户端 App直接播放云盘内容
- 🚀 流式上传 - 边接收边推送到云端，不在本地落盘。上传数十 GB 的文件也不会产生本地缓存，客户端并发数可随意设置
- 📁 群晖 Cloud Sync 兼容 - 已针对群晖 Cloud Sync（含客户端加密）实测调优，大文件同步不再出现超时不断重试上传失败的问题。



## 二进制安装

### 从 GitHub Releases 下载

[Releases](https://github.com/zmzhuai/quarkdrive-webdav/releases) 页面提供 Linux、macOS、Windows 六个平台的预构建包。

### 从源码构建

已装 [Rust](https://www.rust-lang.org/tools/install) 工具链的话：

```bash
cargo install --git https://github.com/zmzhuai/quarkdrive-webdav
```

## 命令行启动

```bash
quarkdrive-webdav --quark-cookie '你的cookie' -U '用户名' -W '密码' -p 8080
```


## Docker 

镜像发布在 `ghcr.io/zmzhuai/quarkdrive-webdav`，支持 amd64 与 arm64。`:latest` 跟随最新一次发版。

也可以用仓库根目录的 `Dockerfile` 自己构建 —— 它从源码编译，产物是静态 musl 二进制，能跑在 DSM 7 这种 4.4 内核上：

```bash
docker build -t quarkdrive-webdav:local .
```

### docker run
```bash
docker run -d --name=quarkdrive-webdav --restart=unless-stopped -p 8080:8080 \
  -e QUARK_COOKIE='your quark cookie' \
  -e WEBDAV_AUTH_USER=admin \
  -e WEBDAV_AUTH_PASSWORD=admin \
  ghcr.io/zmzhuai/quarkdrive-webdav:latest
```

### docker compose

```yaml
version: '3.8'
services:
  quarkdrive-webdav:
    image: ghcr.io/zmzhuai/quarkdrive-webdav:latest
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

已针对群晖 Cloud Sync 实测（含客户端加密、双向同步），文件大小从 KB 级字幕到 4.76GB 视频。加密只改变文件内容与大小，不改变文件名，`Content-Length` 始终存在，因此走流式路径。

Cloud Sync 会在 PUT 刚返回时立刻回头查这个文件，所以它对"上传成功但目录列表还没更新"零容忍 —— 表现是同步日志里偶发的**"上传失败。未找到远程文件"**。上传完成后确认文件可见再返回，就是为这个场景加的。

需要注意的是，**Cloud Sync 的客户端加密使同一文件每次加密的结果都不同**，云端秒传因而无法命中，每次重新同步都是全量重传。这是加密本身的性质，流式上传使其只消耗带宽，不再消耗本地磁盘。

## 发版

1. 改 `Cargo.toml` 里的版本号并提交
2. 建 Release —— 网页上点「Draft a new release」新建一个 `v1.4.0` 的 tag，或者：

```bash
gh release create v1.4.0 --generate-notes
```

发布这一下会触发两个 workflow：多架构镜像推到 ghcr（同时更新 `:latest`），六个平台的二进制挂到这个 Release 上。Release 的标题和正文由你创建时决定，CI 不碰。

两个 workflow 都会先校验 tag 与 `Cargo.toml` 的版本号一致、跑一遍测试，对不上就中止 —— 免得发出去的二进制 `--version` 报的是别的版本。

## 🚨 免责声明

本项目仅供学习和研究目的，不得用于任何商业活动。用户在使用本项目时应遵守所在地区的法律法规，对于违法使用所导致的后果，本项目及作者不承担任何责任。
本项目可能存在未知的缺陷和风险（包括但不限于设备损坏和账号封禁等），使用者应自行承担使用本项目所产生的所有风险及责任。
作者不保证本项目的准确性、完整性、及时性、可靠性，也不承担任何因使用本项目而产生的任何损失或损害责任。
使用本项目即表示您已阅读并同意本免责声明的全部内容。
