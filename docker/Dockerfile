# 运行阶段：极简 Alpine 镜像
# 二进制文件由 CI 预先交叉编译，通过 TARGETARCH 自动选择对应架构
# 需要从源码构建的话，用仓库根目录的 Dockerfile
FROM alpine:3.19

ARG TARGETARCH

RUN addgroup -S app && adduser -S app -G app -s /bin/sh
WORKDIR /app

COPY bin/${TARGETARCH}/quarkdrive-webdav /usr/local/bin/quarkdrive-webdav
RUN chmod +x /usr/local/bin/quarkdrive-webdav

# 暂存目录，仅在客户端不发送 Content-Length 时使用；声明了长度的上传会直接
# 流式推到云端，不落盘。刻意不用 /tmp —— 在很多宿主上它是 tmpfs（即内存），
# 几个并发的大文件上传足以把整台机器拖垮。需要时把卷挂到这个路径。
ENV UPLOAD_TEMP_DIR=/var/cache/quarkdrive
RUN mkdir -p /var/cache/quarkdrive && chown app:app /var/cache/quarkdrive

# 这里原先跑 crond 定时删除 /tmp 中 15 分钟未访问的文件，用来兜住客户端放弃
# 上传时泄漏的暂存文件。该泄漏已在源头修复，而这条定时任务本身是有害的：
# 正在上传的大文件只会被【读取】，relatime 下 atime 不更新，于是它的时间戳
# 会一直是旧的，任务会在传输中途把文件删掉。

USER app
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/quarkdrive-webdav"]
