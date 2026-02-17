/**
 * 写一段脚本，从地址https://github.com/yuemingruoan/better-codex/releases/latest
 * 下载最新版本的程序，并解压。
 * 此程序为命令行程序，用node update.js运行。
 * 下载文件需要手写下载函数。
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

"use strict";

import fs from "node:fs";
import fsp from "node:fs/promises";
import http from "node:http";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { pipeline } from "node:stream/promises";

const RELEASE_LATEST_URL =
  "https://github.com/yuemingruoan/better-codex/releases/latest";

const PLATFORM_OPTIONS = {
  linux: {
    archiveName: "codex-linux.tar.gz",
    archiveType: "tar.gz",
    binaryName: "codex-linux",
  },
  macos: {
    archiveName: "codex-macos.tar.gz",
    archiveType: "tar.gz",
    binaryName: "codex-macos",
  },
  windows: {
    archiveName: "codex-windows.zip",
    archiveType: "zip",
    binaryName: "codex-windows.exe",
  },
} as const;

function parsePlatform(argv: string[]): "linux" | "macos" | "windows" {
  let platform = "linux";

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") {
      printUsage();
      process.exit(0);
    }

    if (arg.startsWith("--platform=")) {
      platform = arg.slice("--platform=".length).trim();
      continue;
    }

    if (arg === "--platform") {
      const nextArg = argv[index + 1];
      if (!nextArg) {
        throw new Error("--platform 缺少参数，例如 --platform=linux");
      }
      platform = nextArg.trim();
      index += 1;
      continue;
    }

    throw new Error(`不支持的参数: ${arg}`);
  }

  const normalizedPlatform = normalizePlatform(platform);
  if (!normalizedPlatform) {
    throw new Error(`不支持的平台: ${platform}`);
  }

  return normalizedPlatform;
}

function normalizePlatform(
  platform: string,
): "linux" | "macos" | "windows" | null {
  const value = platform.trim().toLowerCase();

  switch (value) {
    case "linux":
      return "linux";
    case "mac":
    case "macos":
    case "darwin":
      return "macos";
    case "win":
    case "win32":
    case "windows":
      return "windows";
    default:
      return null;
  }
}

function printUsage() {
  console.log("用法: node scripts/update.js [--platform=linux|macos|windows]");
}

function runCommand(command: string, args: string[]): string {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    const stderr = (result.stderr || "").trim();
    const stdout = (result.stdout || "").trim();
    const message = stderr || stdout || `${command} 执行失败`;
    throw new Error(message);
  }

  return (result.stdout || "").trim();
}

/**
 * @returns {string}
 */
function findCodexBinaryPath() {
  const command = process.platform === "win32" ? "where" : "which";
  const result = runCommand(command, ["codex"]);
  const foundPath = result
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .find(Boolean);

  if (!foundPath) {
    throw new Error("未找到 codex 可执行文件，请先确认 codex 在 PATH 中");
  }

  const externalResolvedPath = resolvePathWithExternalTools(foundPath);
  if (externalResolvedPath) {
    return externalResolvedPath;
  }

  return resolvePathWithReadlink(foundPath);
}

function resolvePathWithExternalTools(filePath: string) {
  if (process.platform === "win32") {
    return null;
  }

  const candidates = [
    ["realpath", [filePath]],
    ["readlink", ["-f", filePath]],
  ] as Array<[string, string[]]>;

  for (const [command, args] of candidates) {
    try {
      const resolvedPath = runCommand(command, args)
        .split(/\r?\n/u)
        .map((line) => line.trim())
        .find(Boolean);

      if (resolvedPath) {
        return resolvedPath;
      }
    } catch {
      // 继续尝试下一个命令
    }
  }

  return null;
}

function resolvePathWithReadlink(filePath: string): string {
  let resolvedPath = path.resolve(filePath);

  for (let depth = 0; depth < 64; depth += 1) {
    const stat = fs.lstatSync(resolvedPath);
    if (!stat.isSymbolicLink()) {
      try {
        return fs.realpathSync(resolvedPath);
      } catch {
        return resolvedPath;
      }
    }

    const linkTarget = fs.readlinkSync(resolvedPath);
    resolvedPath = path.resolve(path.dirname(resolvedPath), linkTarget);
  }

  throw new Error(`符号链接层级过深: ${filePath}`);
}

type DownloadProgressReporter = {
  onChunk: (size: number) => void;
  complete: () => void;
  abort: () => void;
};

function parseContentLength(
  contentLengthHeader: string | string[] | undefined,
): number | null {
  const contentLengthValue = Array.isArray(contentLengthHeader)
    ? contentLengthHeader[0]
    : contentLengthHeader;
  if (!contentLengthValue) {
    return null;
  }

  const contentLength = Number(contentLengthValue);
  if (!Number.isFinite(contentLength) || contentLength <= 0) {
    return null;
  }

  return contentLength;
}

function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"] as const;
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const fractionDigits =
    unitIndex === 0 ? 0 : value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(fractionDigits)} ${units[unitIndex]}`;
}

function createAsciiProgressBar(percent: number, width: number): string {
  const clampedPercent = Math.max(0, Math.min(percent, 100));
  const filledWidth =
    clampedPercent >= 100 ? width : Math.floor((clampedPercent / 100) * width);
  const emptyWidth = Math.max(0, width - filledWidth);
  return `[${"#".repeat(filledWidth)}${"-".repeat(emptyWidth)}]`;
}

function createIndeterminateAsciiBar(frame: number, width: number): string {
  if (width <= 1) {
    return "[>]";
  }

  const cursorPosition = frame % width;
  return `[${"-".repeat(cursorPosition)}>${"-".repeat(width - cursorPosition - 1)}]`;
}

function createDownloadProgressReporter(
  totalBytes: number | null,
): DownloadProgressReporter {
  const isTty = Boolean(process.stdout.isTTY);
  const startedAt = Date.now();
  const progressBarWidth = 30;
  let downloadedBytes = 0;
  let indeterminateFrame = 0;
  let lastRenderAt = 0;
  let inlineLineLength = 0;
  let lastPercentLog = 0;
  let nextByteLogThreshold = 5 * 1024 * 1024;

  const getProgressMessage = (): {
    message: string;
    percent: number | null;
  } => {
    const elapsedSeconds = Math.max((Date.now() - startedAt) / 1000, 0.001);
    const speed = `${formatBytes(downloadedBytes / elapsedSeconds)}/s`;

    if (totalBytes) {
      const percent = Math.min((downloadedBytes / totalBytes) * 100, 100);
      const progressBar = createAsciiProgressBar(percent, progressBarWidth);
      const message = `下载进度 / Download: ${progressBar} ${percent.toFixed(1)}% (${formatBytes(downloadedBytes)}/${formatBytes(totalBytes)}, ${speed})`;
      return { message, percent };
    }

    const progressBar = createIndeterminateAsciiBar(
      indeterminateFrame,
      progressBarWidth,
    );
    indeterminateFrame += 1;
    const message = `下载进度 / Download: ${progressBar} ?? (${formatBytes(downloadedBytes)}, ${speed})`;
    return { message, percent: null };
  };

  const render = (force: boolean) => {
    const now = Date.now();
    if (!force && now - lastRenderAt < 120) {
      return;
    }
    lastRenderAt = now;

    const { message, percent } = getProgressMessage();

    if (isTty) {
      const trailingSpaces = " ".repeat(
        Math.max(0, inlineLineLength - message.length),
      );
      process.stdout.write(`\r${message}${trailingSpaces}`);
      inlineLineLength = message.length;
      return;
    }

    if (percent !== null) {
      if (!force && percent < lastPercentLog + 10) {
        return;
      }
      console.log(message);
      lastPercentLog = Math.floor(percent / 10) * 10;
      return;
    }

    if (!force && downloadedBytes < nextByteLogThreshold) {
      return;
    }
    console.log(message);
    while (downloadedBytes >= nextByteLogThreshold) {
      nextByteLogThreshold += 5 * 1024 * 1024;
    }
  };

  return {
    onChunk: (size: number) => {
      downloadedBytes += size;
      render(false);
    },
    complete: () => {
      render(true);
      if (isTty) {
        process.stdout.write("\n");
      }
    },
    abort: () => {
      if (isTty && inlineLineLength > 0) {
        process.stdout.write("\n");
      }
    },
  };
}

async function downloadFile(
  url: string,
  destinationPath: string,
  redirectCount = 0,
): Promise<void> {
  if (redirectCount > 10) {
    throw new Error(`下载重定向次数过多: ${url}`);
  }

  const parsedUrl = new URL(url);
  const client = parsedUrl.protocol === "https:" ? https : http;

  await new Promise<void>((resolve, reject) => {
    const request = client.get(
      parsedUrl,
      {
        headers: {
          "User-Agent": "better-codex-update-script",
          "Accept": "application/octet-stream",
        },
      },
      async (response) => {
        const statusCode = response.statusCode ?? 0;
        const location = response.headers.location;

        if (statusCode >= 300 && statusCode < 400 && location) {
          response.resume();
          const nextUrl = new URL(location, parsedUrl).toString();
          try {
            await downloadFile(nextUrl, destinationPath, redirectCount + 1);
            resolve();
          } catch (error) {
            reject(error);
          }
          return;
        }

        if (statusCode !== 200) {
          response.resume();
          reject(new Error(`下载失败，HTTP 状态码: ${statusCode}`));
          return;
        }

        const totalBytes = parseContentLength(
          response.headers["content-length"],
        );
        const progressReporter = createDownloadProgressReporter(totalBytes);
        response.on("data", (chunk: Buffer | string) => {
          const chunkSize =
            typeof chunk === "string" ? Buffer.byteLength(chunk) : chunk.length;
          progressReporter.onChunk(chunkSize);
        });

        try {
          await pipeline(response, fs.createWriteStream(destinationPath));
          progressReporter.complete();
          resolve();
        } catch (error) {
          progressReporter.abort();
          await safeRemove(destinationPath);
          reject(error);
        }
      },
    );

    request.on("error", async (error) => {
      await safeRemove(destinationPath);
      reject(error);
    });
  });
}

function extractArchive(
  archivePath: string,
  extractDir: string,
  archiveType: "tar.gz" | "zip",
) {
  if (archiveType === "tar.gz") {
    runCommand("tar", ["-xzf", archivePath, "-C", extractDir]);
    return;
  }

  runCommand("unzip", ["-o", archivePath, "-d", extractDir]);
}

async function findExtractedBinary(
  extractDir: string,
  preferredName: string,
): Promise<string> {
  const preferredPath = path.join(extractDir, preferredName);
  if (await pathExists(preferredPath)) {
    return preferredPath;
  }

  const entries = await fsp.readdir(extractDir, { withFileTypes: true });
  const matchedEntry = entries.find(
    (entry) => entry.isFile() && entry.name.startsWith("codex"),
  );

  if (!matchedEntry) {
    throw new Error("解压后未找到 codex 可执行文件");
  }

  return path.join(extractDir, matchedEntry.name);
}

async function installBinary(
  targetPath: string,
  downloadedBinaryPath: string,
): Promise<string> {
  const targetDir = path.dirname(targetPath);
  const backupPath = await createBackupPath(targetDir);
  const stagedPath = path.join(
    targetDir,
    `${path.basename(targetPath)}.new.${process.pid}`,
  );

  await fsp.copyFile(downloadedBinaryPath, stagedPath);
  await setExecutableIfNeeded(stagedPath);

  let movedOldBinary = false;
  try {
    await fsp.rename(targetPath, backupPath);
    movedOldBinary = true;
    await fsp.rename(stagedPath, targetPath);
  } catch (error) {
    await safeRemove(stagedPath);
    if (movedOldBinary) {
      await safeRename(backupPath, targetPath);
    }
    throw error;
  }

  return backupPath;
}

async function createBackupPath(directory: string): Promise<string> {
  const baseName = formatTimestamp(new Date());
  let candidatePath = path.join(directory, baseName);
  let suffix = 1;

  while (await pathExists(candidatePath)) {
    candidatePath = path.join(
      directory,
      `${baseName}_${String(suffix).padStart(2, "0")}`,
    );
    suffix += 1;
  }

  return candidatePath;
}

function formatTimestamp(date: Date): string {
  const year = String(date.getFullYear());
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hour = String(date.getHours()).padStart(2, "0");
  const minute = String(date.getMinutes()).padStart(2, "0");
  const second = String(date.getSeconds()).padStart(2, "0");
  return `${year}_${month}_${day}_${hour}_${minute}_${second}`;
}

async function pathExists(filePath: string): Promise<boolean> {
  try {
    await fsp.access(filePath, fs.constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

async function setExecutableIfNeeded(filePath: string): Promise<void> {
  if (process.platform === "win32") {
    return;
  }

  await fsp.chmod(filePath, 0o755);
}

async function safeRename(
  sourcePath: string,
  targetPath: string,
): Promise<void> {
  try {
    await fsp.rename(sourcePath, targetPath);
  } catch {
    // 忽略回滚过程中的错误
  }
}

async function safeRemove(filePath: string): Promise<void> {
  try {
    await fsp.rm(filePath, { force: true });
  } catch {
    // 忽略清理过程中的错误
  }
}

async function main() {
  const platform = parsePlatform(process.argv.slice(2));
  const platformOption = PLATFORM_OPTIONS[platform];

  console.log(`目标平台: ${platform}`);
  const codexPath = findCodexBinaryPath();
  console.log(`真实 codex 路径: ${codexPath}`);

  const tempDir = await fsp.mkdtemp(
    path.join(os.tmpdir(), "better-codex-update-"),
  );

  try {
    const archivePath = path.join(tempDir, platformOption.archiveName);
    const extractDir = path.join(tempDir, "extract");
    await fsp.mkdir(extractDir, { recursive: true });

    const downloadUrl = `${RELEASE_LATEST_URL}/download/${platformOption.archiveName}`;
    console.log(`开始下载: ${downloadUrl}`);
    await downloadFile(downloadUrl, archivePath);

    console.log("下载完成，正在解压...");
    extractArchive(archivePath, extractDir, platformOption.archiveType);

    const extractedBinaryPath = await findExtractedBinary(
      extractDir,
      platformOption.binaryName,
    );

    console.log("解压完成，正在替换旧版本...");
    const backupPath = await installBinary(codexPath, extractedBinaryPath);

    console.log("更新成功");
    console.log(`旧版本备份文件: ${backupPath}`);
    console.log(`新版本路径: ${codexPath}`);
  } finally {
    await fsp.rm(tempDir, { recursive: true, force: true });
  }
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`更新失败: ${message}`);
  process.exit(1);
});
