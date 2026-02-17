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

import axios from "axios";
import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { pipeline } from "node:stream/promises";

const RELEASE_LATEST_URL =
  "https://github.com/yuemingruoan/better-codex/releases/latest";
const DOWNLOAD_TIMEOUT_MS = 30_000;

type Platform = "linux" | "macos" | "windows";

type PlatformOption = {
  archiveName: string;
  archiveType: "tar.gz" | "zip";
  binaryCandidates: string[];
};

const PLATFORM_OPTIONS: Record<Platform, PlatformOption> = {
  linux: {
    archiveName: "codex-linux.tar.gz",
    archiveType: "tar.gz",
    binaryCandidates: ["codex-linux", "codex"],
  },
  macos: {
    archiveName: "codex-macos.tar.gz",
    archiveType: "tar.gz",
    binaryCandidates: ["codex-macos", "codex"],
  },
  windows: {
    archiveName: "codex-windows.zip",
    archiveType: "zip",
    binaryCandidates: ["codex-windows.exe", "codex.exe", "codex"],
  },
};

function detectPlatformByRuntime(): Platform {
  switch (process.platform) {
    case "linux":
      return "linux";
    case "darwin":
      return "macos";
    case "win32":
      return "windows";
    default:
      throw new Error(
        `不支持的运行平台 / Unsupported runtime platform: ${process.platform}`,
      );
  }
}

function parsePlatform(argv: string[]): Platform {
  let platform = detectPlatformByRuntime();

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    if (arg === "--help" || arg === "-h") {
      printUsage();
      process.exit(0);
    }

    if (arg.startsWith("--platform=")) {
      platform = normalizePlatform(arg.slice("--platform=".length).trim());
      continue;
    }

    if (arg === "--platform") {
      const nextArg = argv[index + 1];
      if (!nextArg) {
        throw new Error(
          "--platform 缺少参数 / Missing value for --platform, e.g. --platform=linux",
        );
      }
      platform = normalizePlatform(nextArg.trim());
      index += 1;
      continue;
    }

    throw new Error(`不支持的参数 / Unsupported argument: ${arg}`);
  }

  return platform;
}

function normalizePlatform(value: string): Platform {
  switch (value.trim().toLowerCase()) {
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
      throw new Error(`不支持的平台 / Unsupported platform: ${value}`);
  }
}

function printUsage() {
  console.log(
    "用法 / Usage: node scripts/update.ts [--platform=linux|macos|windows]",
  );
  console.log(
    "默认会自动检测当前平台 / Current platform is auto-detected by default.",
  );
}

function runCommand(command: string, args: string[]): string {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    const stderr = (result.stderr || "").trim();
    const stdout = (result.stdout || "").trim();
    const message = stderr || stdout || `${command} 执行失败 / command failed`;
    throw new Error(message);
  }

  return (result.stdout || "").trim();
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

  throw new Error(`符号链接层级过深 / Symlink depth exceeded: ${filePath}`);
}

function findCodexEntryPath(): string {
  const command = process.platform === "win32" ? "where" : "which";
  const result = runCommand(command, ["codex"]);
  const firstPath = result
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .find(Boolean);

  if (!firstPath) {
    throw new Error(
      "未找到 codex 命令 / codex command is not in PATH. Please install codex first.",
    );
  }

  return firstPath;
}

function isLikelyNodeScript(filePath: string): boolean {
  const ext = path.extname(filePath).toLowerCase();
  if (ext === ".js" || ext === ".mjs" || ext === ".cjs") {
    return true;
  }

  try {
    const fd = fs.openSync(filePath, "r");
    try {
      const buffer = Buffer.alloc(160);
      const bytes = fs.readSync(fd, buffer, 0, buffer.length, 0);
      const head = buffer.toString("utf8", 0, bytes).toLowerCase();
      return head.startsWith("#!/") && head.includes("node");
    } finally {
      fs.closeSync(fd);
    }
  } catch {
    return false;
  }
}

function isExecutableFile(filePath: string): boolean {
  try {
    const stat = fs.statSync(filePath);
    if (!stat.isFile()) {
      return false;
    }

    if (process.platform === "win32") {
      return true;
    }

    return (stat.mode & 0o111) !== 0;
  } catch {
    return false;
  }
}

function getTargetTriple(): string | null {
  const { platform, arch } = process;

  switch (platform) {
    case "linux":
    case "android":
      switch (arch) {
        case "x64":
          return "x86_64-unknown-linux-musl";
        case "arm64":
          return "aarch64-unknown-linux-musl";
        default:
          return null;
      }
    case "darwin":
      switch (arch) {
        case "x64":
          return "x86_64-apple-darwin";
        case "arm64":
          return "aarch64-apple-darwin";
        default:
          return null;
      }
    case "win32":
      switch (arch) {
        case "x64":
          return "x86_64-pc-windows-msvc";
        case "arm64":
          return "aarch64-pc-windows-msvc";
        default:
          return null;
      }
    default:
      return null;
  }
}

function collectBinaryCandidates(
  rootDir: string,
  binaryName: string,
): string[] {
  const found: string[] = [];
  const stack: string[] = [rootDir];

  while (stack.length > 0) {
    const currentDir = stack.pop();
    if (!currentDir) {
      continue;
    }

    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(currentDir, { withFileTypes: true });
    } catch {
      continue;
    }

    for (const entry of entries) {
      const fullPath = path.join(currentDir, entry.name);
      if (entry.isDirectory()) {
        stack.push(fullPath);
        continue;
      }

      if (
        (entry.isFile() || entry.isSymbolicLink()) &&
        entry.name === binaryName
      ) {
        found.push(fullPath);
      }
    }
  }

  return found;
}

function resolveBinaryFromNodeWrapper(wrapperPath: string): string | null {
  const targetTriple = getTargetTriple();
  const codexBinaryName = process.platform === "win32" ? "codex.exe" : "codex";

  let currentDir = path.dirname(wrapperPath);
  for (let depth = 0; depth < 8; depth += 1) {
    const vendorDir = path.join(currentDir, "vendor");
    if (isDirectory(vendorDir)) {
      if (targetTriple) {
        const expectedPath = path.join(
          vendorDir,
          targetTriple,
          "codex",
          codexBinaryName,
        );
        if (
          isExecutableFile(expectedPath) &&
          !isLikelyNodeScript(expectedPath)
        ) {
          return expectedPath;
        }
      }

      const candidates = collectBinaryCandidates(vendorDir, codexBinaryName)
        .filter((candidate) => isExecutableFile(candidate))
        .filter((candidate) => !isLikelyNodeScript(candidate));

      if (candidates.length > 0 && targetTriple) {
        const preferred = candidates.find((candidate) =>
          candidate.includes(`${path.sep}${targetTriple}${path.sep}`),
        );
        if (preferred) {
          return preferred;
        }
      }

      if (candidates.length > 0) {
        return candidates[0];
      }
    }

    const parentDir = path.dirname(currentDir);
    if (parentDir === currentDir) {
      break;
    }
    currentDir = parentDir;
  }

  return null;
}

function findCodexBinaryPath(): string {
  const entryPath = findCodexEntryPath();
  const resolvedPath = resolvePathWithReadlink(entryPath);

  if (!isLikelyNodeScript(resolvedPath) && isExecutableFile(resolvedPath)) {
    return resolvedPath;
  }

  const binaryFromWrapper = resolveBinaryFromNodeWrapper(resolvedPath);
  if (binaryFromWrapper) {
    return binaryFromWrapper;
  }

  throw new Error(
    `找到了 codex 入口但无法定位二进制文件 / Found codex entry (${resolvedPath}) but failed to locate native codex binary`,
  );
}

function isDirectory(filePath: string): boolean {
  try {
    return fs.statSync(filePath).isDirectory();
  } catch {
    return false;
  }
}

type DownloadProgressReporter = {
  onChunk: (size: number) => void;
  complete: () => void;
  abort: () => void;
};

function parseContentLength(value: unknown): number | null {
  if (typeof value === "number") {
    return Number.isFinite(value) && value > 0 ? value : null;
  }

  if (typeof value === "string") {
    const parsed = Number(value);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  }

  if (Array.isArray(value) && value.length > 0) {
    return parseContentLength(value[0]);
  }

  return null;
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
  const clamped = Math.max(0, Math.min(percent, 100));
  const filled = clamped >= 100 ? width : Math.floor((clamped / 100) * width);
  const empty = Math.max(0, width - filled);
  return `[${"#".repeat(filled)}${"-".repeat(empty)}]`;
}

function createIndeterminateAsciiBar(frame: number, width: number): string {
  if (width <= 1) {
    return "[>]";
  }

  const cursor = frame % width;
  return `[${"-".repeat(cursor)}>${"-".repeat(width - cursor - 1)}]`;
}

function createDownloadProgressReporter(
  totalBytes: number | null,
): DownloadProgressReporter {
  const isTty = Boolean(process.stdout.isTTY);
  const startedAt = Date.now();
  const barWidth = 30;
  let downloadedBytes = 0;
  let indeterminateFrame = 0;
  let lastRenderAt = 0;
  let lastLineLength = 0;
  let lastPercentLog = 0;
  let nextBytesLog = 5 * 1024 * 1024;

  const getMessage = (): { message: string; percent: number | null } => {
    const elapsedSeconds = Math.max((Date.now() - startedAt) / 1000, 0.001);
    const speed = `${formatBytes(downloadedBytes / elapsedSeconds)}/s`;

    if (totalBytes) {
      const percent = Math.min((downloadedBytes / totalBytes) * 100, 100);
      const bar = createAsciiProgressBar(percent, barWidth);
      const message = `下载进度 / Download: ${bar} ${percent.toFixed(1)}% (${formatBytes(downloadedBytes)}/${formatBytes(totalBytes)}, ${speed})`;
      return { message, percent };
    }

    const bar = createIndeterminateAsciiBar(indeterminateFrame, barWidth);
    indeterminateFrame += 1;
    const message = `下载进度 / Download: ${bar} ?? (${formatBytes(downloadedBytes)}, ${speed})`;
    return { message, percent: null };
  };

  const render = (force: boolean) => {
    const now = Date.now();
    if (!force && now - lastRenderAt < 120) {
      return;
    }
    lastRenderAt = now;

    const { message, percent } = getMessage();

    if (isTty) {
      const spaces = " ".repeat(Math.max(0, lastLineLength - message.length));
      process.stdout.write(`\r${message}${spaces}`);
      lastLineLength = message.length;
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

    if (!force && downloadedBytes < nextBytesLog) {
      return;
    }

    console.log(message);
    while (downloadedBytes >= nextBytesLog) {
      nextBytesLog += 5 * 1024 * 1024;
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
      if (isTty && lastLineLength > 0) {
        process.stdout.write("\n");
      }
    },
  };
}

async function downloadFile(
  url: string,
  destinationPath: string,
): Promise<void> {
  const response = await axios.get(url, {
    responseType: "stream",
    timeout: DOWNLOAD_TIMEOUT_MS,
    maxRedirects: 10,
    headers: {
      "User-Agent": "better-codex-update-script",
      "Accept": "application/octet-stream",
    },
  });

  if (response.status !== 200) {
    response.data.destroy();
    throw new Error(
      `下载失败 / Download failed with status: ${response.status}`,
    );
  }

  const totalBytes = parseContentLength(response.headers["content-length"]);
  const progress = createDownloadProgressReporter(totalBytes);

  response.data.on("data", (chunk: Buffer | string) => {
    const chunkSize =
      typeof chunk === "string" ? Buffer.byteLength(chunk) : chunk.length;
    progress.onChunk(chunkSize);
  });

  try {
    await pipeline(response.data, fs.createWriteStream(destinationPath));
    progress.complete();
  } catch (error) {
    progress.abort();
    await safeRemove(destinationPath);
    throw error;
  }
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
  binaryCandidates: string[],
): Promise<string> {
  const targetNames = new Set(
    binaryCandidates.map((name) => name.toLowerCase()),
  );
  const queue = [extractDir];
  let fallbackPath: string | null = null;

  while (queue.length > 0) {
    const currentDir = queue.shift();
    if (!currentDir) {
      continue;
    }

    const entries = await fsp.readdir(currentDir, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(currentDir, entry.name);

      if (entry.isDirectory()) {
        queue.push(fullPath);
        continue;
      }

      if (!entry.isFile() && !entry.isSymbolicLink()) {
        continue;
      }

      const normalizedName = entry.name.toLowerCase();
      if (targetNames.has(normalizedName)) {
        return fullPath;
      }

      if (
        !fallbackPath &&
        normalizedName.startsWith("codex") &&
        !normalizedName.endsWith(".js") &&
        !normalizedName.endsWith(".ts")
      ) {
        fallbackPath = fullPath;
      }
    }
  }

  if (fallbackPath) {
    return fallbackPath;
  }

  throw new Error(
    "解压后未找到 codex 可执行文件 / codex binary was not found after extraction",
  );
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
  let candidate = path.join(directory, baseName);
  let suffix = 1;

  while (await pathExists(candidate)) {
    candidate = path.join(
      directory,
      `${baseName}_${String(suffix).padStart(2, "0")}`,
    );
    suffix += 1;
  }

  return candidate;
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
    // ignore rollback failures
  }
}

async function safeRemove(filePath: string): Promise<void> {
  try {
    await fsp.rm(filePath, { force: true, recursive: true });
  } catch {
    // ignore cleanup failures
  }
}

async function main() {
  const platform = parsePlatform(process.argv.slice(2));
  const platformOption = PLATFORM_OPTIONS[platform];

  console.log(`目标平台 / Platform: ${platform}`);

  const codexBinaryPath = findCodexBinaryPath();
  console.log(`已定位 codex 二进制 / Located codex binary: ${codexBinaryPath}`);

  const tempDir = await fsp.mkdtemp(
    path.join(os.tmpdir(), "better-codex-update-"),
  );

  try {
    const archivePath = path.join(tempDir, platformOption.archiveName);
    const extractDir = path.join(tempDir, "extract");
    await fsp.mkdir(extractDir, { recursive: true });

    const downloadUrl = `${RELEASE_LATEST_URL}/download/${platformOption.archiveName}`;
    console.log(`开始下载 / Start download: ${downloadUrl}`);
    await downloadFile(downloadUrl, archivePath);

    console.log("下载完成，开始解压 / Download complete, extracting...");
    extractArchive(archivePath, extractDir, platformOption.archiveType);

    const extractedBinaryPath = await findExtractedBinary(
      extractDir,
      platformOption.binaryCandidates,
    );

    console.log("解压完成，开始安装 / Extraction complete, installing...");
    const backupPath = await installBinary(
      codexBinaryPath,
      extractedBinaryPath,
    );

    console.log("更新成功 / Update completed successfully");
    console.log(`旧版本备份 / Backup: ${backupPath}`);
    console.log(`新版本路径 / New binary path: ${codexBinaryPath}`);
  } finally {
    await fsp.rm(tempDir, { recursive: true, force: true });
  }
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  console.error(`更新失败 / Update failed: ${message}`);
  process.exit(1);
});
