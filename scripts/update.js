/**
 * 写一段脚本，从地址https://github.com/yuemingruoan/better-codex/releases/latest
 * 下载最新版本的程序，并解压。
 * 此程序为命令行程序，用node update.js运行。
 * 可以接收参数，node update.js --platform=?? 指定下载某个平台的版本
 * 包含windows、linux、macos三个平台，默认下载linux版本
 * 下载完成后，通过which codex
 * 或者
 * realpath "$(which codex)"            # GNU 系统
 * readlink -f "$(which codex)"         # Linux
 * readlink "$(which codex)"            # macOS 需逐级解析
 * 视情况而定来找到真正的codex位置。
 * 再对真正的codex二进制文件改名，改为当前年月日时分秒，用下划线链接，不足2位数左侧补0。
 * 然后将解压过来的文件命名为codex放在原位。
 */
