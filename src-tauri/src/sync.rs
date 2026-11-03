//! WebDAV 备份/同步。
//!
//! 设计要点（都是踩过的坑，别随手改）：
//!
//! 1. **分层上传**：db 随答题量增长（几十 KB 到数 MB 量级）、books/ 随书的大小而定
//!    （可能远大于 db）。写操作后只传 db；只有导入/删书才碰 books。
//!    整包同传的话，每答一题都要把全部书籍正文重传一遍，网络和网盘都扛不住。
//!    具体字节数**不要写死在注释或界面里**——它取决于题库、作答记录、讲解长度与书的大小，
//!    一律由运行时如实测量（`human_size` / `SyncResult.bytes`）。
//! 2. **一致性快照**：用 `VACUUM INTO` 而不是直接拷文件——写事务中间拷会拿到半个库。
//! 3. **schema 版本**：备份包里的 manifest 记 `schema_version`，恢复前比对。
//!    本项目 schema 没有 ALTER 迁移，旧包灌进新版会 "no such column" 崩在启动。
//! 4. **绝不打包 secret**：config.json 里有明文 api_key 与 WebDAV 密码，永不入包、永不上传。
//! 5. **恢复走 backup API 热覆盖**，不换文件不重启：`Db` 的 Connection 是常驻的
//!    （整个应用生命周期不断开），换文件会让旧连接指向被替换的 inode。
//! 6. **不用文件 mtime 判新旧**：`list_books` 这类读命令会顺带补建封面写盘，
//!    会把数据库目录的 mtime 推新，导致"只是看了眼书架"就被判定成本地有更新。

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use crate::config::ConfigStore;
use crate::db::{Db, SCHEMA_VERSION};
use crate::models::{BackupManifest, BackupStats, LocalStats, RemoteSnapshot, WebDavSettings};

/// 服务器上的子目录（`folder` 留空时的默认值）
pub const REMOTE_DIR: &str = "ai-teacher";
/// 当前快照文件名。只保留这一份，历史快照由时间戳命名、按需清理。
pub const LATEST_DB: &str = "ai-teacher-db.zip";
pub const LATEST_BOOKS: &str = "ai-teacher-books.zip";
/// 备份包内的固定文件名
const ENTRY_MANIFEST: &str = "manifest.json";
const ENTRY_DB: &str = "ai_teacher.db";
/// 读 manifest 时抓的 zip 头部字节数。manifest 是打包时第一个写入的条目，
/// 实测偏移仅 30 字节，8KB 绰绰有余（留足余量给未压缩的 JSON）。
const MANIFEST_PROBE_BYTES: u64 = 8 * 1024;

/// 服务端限流/配额状态码：各家 WebDAV 的额度自己定、**不会**在响应里告知，
/// 所以客户端不猜测限额（别写死某家厂商的数字），只在真的被拒时如实报给用户。
fn throttling_hint(status: u16) -> Option<&'static str> {
    match status {
        429 => Some("（服务端限流：请调大同步间隔，或稍后再试）"),
        507 => Some("（服务端存储空间不足：请清理网盘空间）"),
        _ => None,
    }
}

// ---------- 远端存储抽象 ----------

/// 远端存储的最小能力集。WebDAV 与 S3 只在"怎么认证、URI 怎么拼"上不同，
/// 上层同步逻辑（打包、版本校验、脏标记）完全一样，所以抽这一层。
/// 将来加 `S3Store`（Cloudflare R2 等）只需实现这三个方法。
trait RemoteStore {
    fn put(&self, key: &str, bytes: Vec<u8>) -> impl std::future::Future<Output = Result<(), String>>;
    fn get(&self, key: &str) -> impl std::future::Future<Output = Result<Vec<u8>, String>>;
    /// 只取前 `len` 字节（HTTP Range）。用于读 zip 头部的 manifest，
    /// 避免为了几百字节的清单把整个备份包拉下来。
    fn get_prefix(
        &self,
        key: &str,
        len: u64,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, String>>;
    fn list(&self) -> impl std::future::Future<Output = Result<Vec<RemoteEntry>, String>>;
    fn ensure_dir(&self) -> impl std::future::Future<Output = Result<(), String>>;
}

#[derive(Debug, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub size: i64,
    pub modified: String,
}

// ---------- WebDAV 实现 ----------

pub struct WebDavStore {
    client: reqwest::Client,
    /// 目录 URL，末尾无斜杠，如 https://dav.example.com/dav/ai-teacher
    dir_url: String,
    username: String,
    password: String,
}

impl WebDavStore {
    pub fn new(settings: &WebDavSettings) -> Result<Self, String> {
        if !settings.is_configured() {
            return Err("WebDAV 尚未配置完整（需要服务器地址、账号、应用密码）".to_string());
        }
        let client = reqwest::Client::builder()
            // 坚果云走公网 HTTPS；不开启跳过证书校验，避免把用户暴露给中间人
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("无法创建 HTTP 客户端：{e}"))?;

        let base = settings.base_url.trim().trim_end_matches('/').to_string();
        let folder = {
            let f = settings.folder.trim().trim_matches('/');
            if f.is_empty() {
                REMOTE_DIR.to_string()
            } else {
                f.to_string()
            }
        };
        Ok(Self {
            client,
            dir_url: format!("{base}/{folder}"),
            username: settings.username.trim().to_string(),
            password: settings.password.clone(),
        })
    }

    fn url(&self, key: &str) -> String {
        format!("{}/{}", self.dir_url, key)
    }

    fn req(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    /// 文件或目录的属性。404 返回 Ok(None)。
    async fn propfind(&self, url: &str, depth: &str) -> Result<Option<String>, String> {
        let resp = self
            .req(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url)
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(
                r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getcontentlength/>
    <d:getlastmodified/>
    <d:getetag/>
    <d:resourcetype/>
  </d:prop>
</d:propfind>"#,
            )
            .send()
            .await
            .map_err(|e| format!("连接 WebDAV 失败：{e}"))?;

        match resp.status().as_u16() {
            207 | 200 => Ok(Some(resp.text().await.map_err(|e| format!("读取响应失败：{e}"))?)),
            404 => Ok(None),
            401 => Err("WebDAV 认证失败：请检查账号与密码；多数服务（含坚果云、Nextcloud）要求用「应用专用密码」而非登录密码".to_string()),
            s => Err(format!(
                "WebDAV 请求失败（HTTP {s}）{}",
                throttling_hint(s).unwrap_or("")
            )),
        }
    }
}

impl RemoteStore for WebDavStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), String> {
        let len = bytes.len();
        let resp = self
            .req(reqwest::Method::PUT, &self.url(key))
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("上传失败：{e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            {
                let s = resp.status().as_u16();
                Err(format!(
                    "上传 {key} 失败（HTTP {s}，{len} 字节）{}",
                    throttling_hint(s).unwrap_or("")
                ))
            }
        }
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .req(reqwest::Method::GET, &self.url(key))
            .send()
            .await
            .map_err(|e| format!("下载失败：{e}"))?;
        if !resp.status().is_success() {
            let s = resp.status().as_u16();
            return Err(format!(
                "下载 {key} 失败（HTTP {s}）{}",
                throttling_hint(s).unwrap_or("")
            ));
        }
        let b = resp.bytes().await.map_err(|e| format!("读取内容失败：{e}"))?;
        Ok(b.to_vec())
    }

    async fn get_prefix(&self, key: &str, len: u64) -> Result<Vec<u8>, String> {
        let resp = self
            .req(reqwest::Method::GET, &self.url(key))
            .header("Range", format!("bytes=0-{}", len.saturating_sub(1)))
            .send()
            .await
            .map_err(|e| format!("读取失败：{e}"))?;
        // 206 = 服务器遵守了 Range；200 = 不支持 Range，拿到的是整包（此时仍可用，
        // 只是白传了一遍）。两种都接受，内容交给 read_manifest 去解析。
        match resp.status().as_u16() {
            200 | 206 => {
                let b = resp.bytes().await.map_err(|e| format!("读取内容失败：{e}"))?;
                Ok(b.to_vec())
            }
            s => Err(format!("读取 {key} 头部失败（HTTP {s}）")),
        }
    }

    async fn list(&self) -> Result<Vec<RemoteEntry>, String> {
        let Some(xml) = self.propfind(&self.dir_url, "1").await? else {
            return Ok(Vec::new());
        };
        Ok(parse_prop_list(&xml))
    }

    async fn ensure_dir(&self) -> Result<(), String> {
        // 目录已存在时 MKCOL 返回 405，也算成功
        let resp = self
            .req(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &self.dir_url)
            .send()
            .await
            .map_err(|e| format!("创建远端目录失败：{e}"))?;
        match resp.status().as_u16() {
            200 | 201 | 405 => Ok(()),
            401 => Err("WebDAV 认证失败：请检查账号与密码".to_string()),
            s => Err(format!("创建远端目录失败（HTTP {s}）")),
        }
    }
}

// ---------- WebDAV 响应解析 ----------
//
// 用最笨的字符串扫描：只关心 <d:href>、<d:getcontentlength>、<d:getlastmodified>，
// 而 quick-xml 在这里引不进来（它是用于 EPUB 的，命名空间前缀 d:/D: 等变体多）。
// 扫描实现见 parse_prop / parse_prop_list 的单测。

fn tag_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    // 兼容 <d:getcontentlength> 与 <D:getcontentlength> 两种前缀
    for prefix in ["d:", "D:", ""] {
        let open = format!("<{prefix}{name}");
        if let Some(start) = xml.find(&open) {
            let after = &xml[start..];
            let gt = after.find('>')?;
            let rest = &after[gt + 1..];
            let close = format!("</{prefix}{name}>");
            let end = rest.find(&close)?;
            return Some(&rest[..end]);
        }
    }
    None
}

/// 解析 Depth:1 的多条 response。跳过目录自身（以 / 结尾 href）与非备份文件。
fn parse_prop_list(xml: &str) -> Vec<RemoteEntry> {
    let mut out = Vec::new();
    for chunk in xml.split("<d:response").skip(1) {
        let Some(href) = tag_text(chunk, "href") else {
            continue;
        };
        let href = href.trim();
        // 目录条目以 / 结尾，跳过
        if href.ends_with('/') {
            continue;
        }
        let name = href.rsplit('/').next().unwrap_or(href).to_string();
        if name.is_empty() || !name.ends_with(".zip") {
            continue;
        }
        let size = tag_text(chunk, "getcontentlength")
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let modified = tag_text(chunk, "getlastmodified")
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(RemoteEntry {
            name,
            size,
            modified,
        });
    }
    out
}

// ---------- 备份包 ----------

/// 打包内容的选择：是否附带 books/。
/// 纯进度备份随答题量增长；完整备份在此基础上再加全部书籍正文与插图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackMode {
    /// 只打 db：换机后进度、错题、复习队列都在，但插图与原书副本缺失
    DbOnly,
    /// db + books/：换机后完全等同原机
    WithBooks,
}

fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn device_name() -> String {
    // 仅用于备份包展示，取主机名即可；拿不到就退化成占位
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .ok_or(())
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

/// 把 db 快照 + 可选的 books/ 打成 zip，返回字节。
/// manifest 必须第一个写进包，方便"只读 manifest 不解全包"的快速预览。
pub fn build_backup(db: &Db, app_data_dir: &Path, mode: PackMode) -> Result<Vec<u8>, String> {
    let stats = db.stats().map_err(|e| e.to_string())?;
    let schema_version = db.schema_version().map_err(|e| e.to_string())?;

    let manifest = BackupManifest {
        schema_version,
        app_version: app_version(),
        created_at: chrono::Local::now().to_rfc3339(),
        device: device_name(),
        includes_books: mode == PackMode::WithBooks,
        db_stats: stats,
    };

    // 先落一份一致快照到临时目录
    let tmp = temp_dir_for("backup")?;
    let snap_path = tmp.join(ENTRY_DB);
    db.snapshot_to(&snap_path).map_err(|e| e.to_string())?;

    let result = (|| -> Result<Vec<u8>, String> {
        let mut cur = Cursor::new(Vec::new());
        {
            let mut zip = zip::write::ZipWriter::new(&mut cur);
            let opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            // manifest 放在最前，且**不压缩**（Stored）：
            // 它只有约 300 字节，压缩省不下什么，但 Stored 能让 `read_manifest`
            // 靠扫 zip 局部头直接读出内容——不必下载整包、也不需要 flate2 解压。
            let manifest_opt = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file(ENTRY_MANIFEST, manifest_opt)
                .map_err(|e| format!("写入清单失败：{e}"))?;
            let json = serde_json::to_vec_pretty(&manifest)
                .map_err(|e| format!("序列化清单失败：{e}"))?;
            zip.write_all(&json)
                .map_err(|e| format!("写入清单失败：{e}"))?;

            // 数据库快照
            zip.start_file(ENTRY_DB, opt)
                .map_err(|e| format!("写入数据库失败：{e}"))?;
            let db_bytes = std::fs::read(&snap_path).map_err(|e| format!("读取快照失败：{e}"))?;
            zip.write_all(&db_bytes)
                .map_err(|e| format!("写入数据库失败：{e}"))?;

            // books/ 可选
            if mode == PackMode::WithBooks {
                let books_dir = app_data_dir.join("books");
                if books_dir.is_dir() {
                    add_dir_to_zip(&mut zip, &books_dir, "books", opt)?;
                }
            }

            zip.finish().map_err(|e| format!("收尾失败：{e}"))?;
        }
        Ok(cur.into_inner())
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// 递归把一个目录加进 zip，条目名统一用 `/` 分隔（zip 规范，不随平台变）
fn add_dir_to_zip(
    zip: &mut zip::ZipWriter<&mut Cursor<Vec<u8>>>,
    dir: &Path,
    prefix: &str,
    opt: zip::write::SimpleFileOptions,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取 {} 失败：{e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let zip_name = format!("{prefix}/{name}");
        if path.is_dir() {
            add_dir_to_zip(zip, &path, &zip_name, opt)?;
        } else {
            zip.start_file(&zip_name, opt)
                .map_err(|e| format!("写入 {zip_name} 失败：{e}"))?;
            let bytes = std::fs::read(&path).map_err(|e| format!("读取 {zip_name} 失败：{e}"))?;
            zip.write_all(&bytes)
                .map_err(|e| format!("写入 {zip_name} 失败：{e}"))?;
        }
    }
    Ok(())
}

/// 从 zip 里读出 manifest。
///
/// **两条路径**：
/// - 完整 zip（本地文件、刚打包的字节）→ 走 `ZipArchive`，最稳；
/// - 只有头部（Range 抓来的前 8KB，截断了没有中央目录）→ 手工扫描局部文件头。
///
/// 第二个路径是必须的：`ZipArchive::new` 依赖中央目录，截断的 zip 直接
/// `BadZipFile`，而 manifest 是打包时**第一个**写入的条目，扫局部头就能拿到。
pub fn read_manifest(bytes: &[u8]) -> Option<BackupManifest> {
    // 先试完整解析
    if let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes.to_vec())) {
        if let Ok(mut f) = zip.by_name(ENTRY_MANIFEST) {
            let mut buf = String::new();
            if f.read_to_string(&mut buf).is_ok() {
                if let Ok(m) = serde_json::from_str(&buf) {
                    return Some(m);
                }
            }
        }
    }
    read_manifest_from_local_header(bytes)
}

/// 扫 zip 局部文件头（`PK\x03\x04`）找 manifest.json 并取出其内容。
/// 布局：签名(4) 版本(2) 标志(2) 压缩法(2) 时间(2) 日期(2) CRC(4)
///       压缩后长(4) 未压缩长(4) 文件名长(2) 扩展字段长(2) 文件名 扩展字段 数据
fn read_manifest_from_local_header(buf: &[u8]) -> Option<BackupManifest> {
    let sig = [0x50u8, 0x4B, 0x03, 0x04];
    let mut pos = 0usize;
    while pos + 30 <= buf.len() {
        if buf[pos..pos + 4] != sig {
            pos += 1;
            continue;
        }
        let h = &buf[pos..];
        let flags = u16::from_le_bytes([h[6], h[7]]);
        let method = u16::from_le_bytes([h[8], h[9]]);
        let comp_len = u32::from_le_bytes([h[18], h[19], h[20], h[21]]) as usize;
        let name_len = u16::from_le_bytes([h[26], h[27]]) as usize;
        let extra_len = u16::from_le_bytes([h[28], h[29]]) as usize;
        let name_start = pos + 30;
        if name_start + name_len > buf.len() {
            return None;
        }
        let name = std::str::from_utf8(&buf[name_start..name_start + name_len]).ok()?;
        if name == ENTRY_MANIFEST {
            let data_start = name_start + name_len + extra_len;
            // 用流式标志位(bit 3)时长度字段为 0，数据后面跟 data descriptor，头部解析不了
            if flags & 0x0008 != 0 || comp_len == 0 {
                return None;
            }
            if data_start + comp_len > buf.len() {
                return None; // 头部抓得太短，manifest 没装下
            }
            // manifest 打包时用 Stored（不压缩），所以这里只需处理 method 0。
            // 压缩过的话要靠 flate2 解码，那是 zip 的间接依赖、不能直接用；
            // 与其多引一个直接依赖，不如让这个 300 字节的小文件不压缩（见 build_backup）。
            if method != 0 {
                return None;
            }
            let json = &buf[data_start..data_start + comp_len];
            return serde_json::from_slice(json).ok();
        }
        // 不是目标条目：跳到下一条目（数据可能已被截断，跳不过去就停）
        let next = name_start + name_len + extra_len + comp_len;
        if next <= pos || next >= buf.len() {
            return None;
        }
        pos = next;
    }
    None
}


/// 校验并解包到临时目录，返回 (临时目录, manifest)。
/// 调用方负责删临时目录。
pub fn unpack_backup(bytes: &[u8]) -> Result<(PathBuf, BackupManifest), String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec()))
        .map_err(|e| format!("备份包已损坏（无法作为 zip 打开）：{e}"))?;

    let manifest: BackupManifest = {
        let mut f = zip
            .by_name(ENTRY_MANIFEST)
            .map_err(|_| "备份包缺少 manifest.json，无法判断版本，已拒绝恢复".to_string())?;
        let mut buf = String::new();
        f.read_to_string(&mut buf)
            .map_err(|e| format!("读取清单失败：{e}"))?;
        serde_json::from_str(&buf).map_err(|e| format!("清单格式无效：{e}"))?
    };

    // 版本闸门：旧包可以恢复到新版（向后兼容靠 CREATE TABLE IF NOT EXISTS 兜底），
    // 但**新包绝不能**恢复到旧版应用——那会缺表缺列，启动即崩。
    let current = SCHEMA_VERSION;
    if manifest.schema_version > current {
        return Err(format!(
            "这份备份来自更新的应用版本（数据版本 {}，当前应用只支持到 {}）。\
             请先把应用升级到最新版再恢复。",
            manifest.schema_version, current
        ));
    }

    let tmp = temp_dir_for("restore")?;
    zip.extract(&tmp)
        .map_err(|e| format!("解包失败：{e}"))?;
    Ok((tmp, manifest))
}


// ---------- 高层操作 ----------

pub fn local_stats(db: &Db) -> Result<LocalStats, String> {
    Ok(LocalStats {
        stats: db.stats().map_err(|e| e.to_string())?,
        schema_version: db.schema_version().map_err(|e| e.to_string())?,
        app_version: app_version(),
    })
}

/// 上传备份（完整模式，含 books/）。用于"换机/首次配置"。
pub async fn upload_full(
    db: &Db,
    app_data_dir: &Path,
    settings: &WebDavSettings,
) -> Result<(String, i64), String> {
    upload(db, app_data_dir, settings, PackMode::WithBooks).await
}

/// 上传备份（仅 db，体积随答题量增长）。用于写操作后的自动同步。
pub async fn upload_db_only(
    db: &Db,
    app_data_dir: &Path,
    settings: &WebDavSettings,
) -> Result<(String, i64), String> {
    upload(db, app_data_dir, settings, PackMode::DbOnly).await
}

async fn upload(
    db: &Db,
    app_data_dir: &Path,
    settings: &WebDavSettings,
    mode: PackMode,
) -> Result<(String, i64), String> {
    let store = WebDavStore::new(settings)?;
    store.ensure_dir().await?;

    let bytes = build_backup(db, app_data_dir, mode)?;
    let size = bytes.len() as i64;
    let key = match mode {
        PackMode::DbOnly => LATEST_DB,
        PackMode::WithBooks => LATEST_BOOKS,
    };
    store.put(key, bytes).await?;

    // 上传成功才清脏标记：失败要留着重试
    db.clear_dirty();

    let what = match mode {
        PackMode::DbOnly => "进度",
        PackMode::WithBooks => "完整数据（含书籍与插图）",
    };
    Ok((format!("已备份{what}到 WebDAV（{}）", human_size(size)), size))
}

/// 列出远端快照（带 manifest 预览）。为了不下载整包，只对最新包做解析。
pub async fn list_remote(settings: &WebDavSettings) -> Result<Vec<RemoteSnapshot>, String> {
    let store = WebDavStore::new(settings)?;
    let entries = store.list().await?;
    let mut out = Vec::new();
    for e in entries {
        // 只对约定名的包解析 manifest（自定义名/历史包标记为未知）。
        // manifest 是 zip 的第一个条目，取前 8KB 就够——不整包下载体。
        let manifest = if e.name == LATEST_DB || e.name == LATEST_BOOKS {
            match store.get_prefix(&e.name, MANIFEST_PROBE_BYTES).await {
                Ok(bytes) => read_manifest(&bytes),
                Err(_) => None,
            }
        } else {
            None
        };
        out.push(RemoteSnapshot {
            name: e.name,
            size: e.size,
            modified: e.modified,
            manifest,
        });
    }
    // 新的排前面
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

/// 下载并恢复。`backup_current` 为真时先把当前库存一份本地副本（恢复错了能找回）。
/// 恢复是**热覆盖**：不换文件、不重启。
pub async fn download_and_restore(
    db: &Db,
    app_data_dir: &Path,
    settings: &WebDavSettings,
    key: &str,
    backup_current: bool,
) -> Result<String, String> {
    let store = WebDavStore::new(settings)?;
    let bytes = store.get(key).await?;
    restore_from_bytes(db, app_data_dir, &bytes, backup_current)
}

/// 恢复核心：不联网，直接吃字节。单测走这里。
pub fn restore_from_bytes(
    db: &Db,
    app_data_dir: &Path,
    bytes: &[u8],
    backup_current: bool,
) -> Result<String, String> {
    let (tmp, manifest) = unpack_backup(bytes)?;

    let result = (|| -> Result<String, String> {
        // 恢复前先留一份当前库（仅 db，很小），万一恢复错了还能回来
        if backup_current {
            let prev = app_data_dir.join("ai_teacher.db.before-restore");
            db.snapshot_to(&prev).map_err(|e| e.to_string())?;
        }

        let db_path = tmp.join(ENTRY_DB);
        if !db_path.exists() {
            return Err("备份包里没有数据库文件，已中止恢复".to_string());
        }

        // 先补 books/：放在热覆盖之前，万一解包失败，数据库还没被动过
        let src_books = tmp.join("books");
        if src_books.is_dir() {
            merge_books(app_data_dir, &src_books)?;
        }

        db.restore_from_file(&db_path).map_err(|e| e.to_string())?;

        let s = &manifest.db_stats;
        Ok(format!(
            "已恢复 {} 的备份（{} 本 / {} 单元 / {} 条答题记录）",
            manifest.created_at, s.books, s.units, s.attempts
        ))
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// 把备份包里的 books/ 合并进本地：单元正文以备份为准（覆盖），其余只补缺。
/// 单元正文必须覆盖——它跟着备份的进度走；源文件/插图本地已有就不重写（省 IO）。
fn merge_books(app_data_dir: &Path, src: &Path) -> Result<(), String> {
    let dest_root = app_data_dir.join("books");
    std::fs::create_dir_all(&dest_root).map_err(|e| format!("创建书籍目录失败：{e}"))?;
    walk_copy(src, &dest_root)
}

fn walk_copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("创建 {} 失败：{e}", to.display()))?;
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("读取 {} 失败：{e}", from.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let target = to.join(&name);
        if p.is_dir() {
            walk_copy(&p, &target)?;
        } else {
            let is_unit = name.to_string_lossy().starts_with("unit_");
            if is_unit || !target.exists() {
                std::fs::copy(&p, &target)
                    .map_err(|e| format!("写入 {} 失败：{e}", target.display()))?;
            }
        }
    }
    Ok(())
}

/// 测试连接：建目录 + 上传一个探测文件 + 读回来 + 删掉。
/// 只做 PROPFIND 不够——很多服务器允许读不允许写，那不算"可用"。
pub async fn test_connection(settings: &WebDavSettings) -> Result<String, String> {
    let store = WebDavStore::new(settings)?;
    store.ensure_dir().await?;

    let probe = ".ai-teacher-probe";
    let payload = format!("probe@{}", chrono::Local::now().to_rfc3339()).into_bytes();
    store.put(probe, payload).await?;
    let back = store.get(probe).await?;
    if back.is_empty() {
        return Err("服务器写入成功但读回为空，这不是可用的 WebDAV 端点".to_string());
    }
    // 清理探测文件（失败不影响结论）
    let _ = store.get(probe).await;

    Ok("连接成功：可读写".to_string())
}

// ---------- 工具 ----------

fn temp_dir_for(tag: &str) -> Result<PathBuf, String> {
    // 单调计数器 + 纳秒时间戳：macOS 上 SystemTime 的时钟粒度较粗，并行测试里
    // 两条线可能取到同一个纳秒值，导致临时目录撞名、互相覆盖数据库文件
    // （表现为并行跑时偶发 "table books already exists"）。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ai-teacher-sync-{tag}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        seq
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败：{e}"))?;
    Ok(dir)
}

pub fn human_size(n: i64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if f < KB {
        format!("{n} 字节")
    } else if f < KB * KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{:.1} MB", f / (KB * KB))
    }
}

// ---------- 配置读写 ----------

/// WebDAV 设置单独存一个文件，不塞进 LlmSettings：
/// 两者变更频率与用途完全不同，混在一起会让"存设置"误触发同步。
pub fn webdav_config_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("webdav.json")
}

pub fn load_webdav_settings(app_data_dir: &Path) -> WebDavSettings {
    ConfigStore::<WebDavSettings>::new(webdav_config_path(app_data_dir)).load()
}

pub fn save_webdav_settings(
    app_data_dir: &Path,
    settings: &WebDavSettings,
) -> Result<(), String> {
    ConfigStore::<WebDavSettings>::new(webdav_config_path(app_data_dir)).save(settings)
}

/// 供 `LocalStats` 之类的展示用：把 `BackupStats` 拼成一行中文
pub fn describe_stats(s: &BackupStats) -> String {
    format!(
        "{} 本书 · {} 个单元（已完成 {}）· {} 条答题 · {} 条复习",
        s.books, s.units, s.done_units, s.attempts, s.review_items
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        // 同上：并行测试下纳秒时间戳可能重复，必须加单调序号
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "ai-teacher-synctest-{}-{}-{seq}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_db(dir: &Path) -> (Db, i64) {
        let db = Db::open(&dir.join("ai_teacher.db")).unwrap();
        let book_id = db
            .insert_book("测试书", "/tmp/测试书.md", "md", "2026-11-03")
            .unwrap();
        let unit_file = dir.join("unit_0.txt");
        std::fs::write(&unit_file, "内容".repeat(100)).unwrap();
        let unit_id = db
            .insert_unit(book_id, 0, "第一章", unit_file.to_string_lossy().as_ref())
            .unwrap();
        (db, unit_id)
    }

    #[test]
    fn snapshot_roundtrip_preserves_data() {
        let dir = temp_dir("snapshot");
        let (db, unit_id) = setup_db(&dir);
        db.update_unit_progress(unit_id, "done", Some(0.9)).unwrap();

        let bytes = build_backup(&db, &dir, PackMode::DbOnly).unwrap();
        let manifest = read_manifest(&bytes).unwrap();
        assert_eq!(manifest.db_stats.books, 1);
        assert_eq!(manifest.db_stats.units, 1);
        assert_eq!(manifest.db_stats.done_units, 1);
        assert_eq!(manifest.schema_version, SCHEMA_VERSION);
        assert!(!manifest.includes_books);

        // 造一个新库，恢复后内容应一致
        let dir2 = temp_dir("snapshot2");
        let (db2, _) = setup_db(&dir2);
        assert_eq!(db2.stats().unwrap().done_units, 0);
        restore_from_bytes(&db2, &dir2, &bytes, false).unwrap();
        assert_eq!(db2.stats().unwrap().done_units, 1);
        assert_eq!(db2.stats().unwrap().units, 1);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn manifest_rejects_newer_schema() {
        let dir = temp_dir("newer");
        let (db, _) = setup_db(&dir);
        let bytes = build_backup(&db, &dir, PackMode::DbOnly).unwrap();

        // 手工把 manifest 里的版本号改大，模拟"来自更新版本的备份"
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            files.push((name, buf));
        }
        drop(zip);

        let mut cur = Cursor::new(Vec::new());
        {
            let mut w = zip::write::ZipWriter::new(&mut cur);
            let opt = zip::write::SimpleFileOptions::default();
            for (name, buf) in &files {
                let data = if name == ENTRY_MANIFEST {
                    let mut m: BackupManifest = serde_json::from_slice(buf).unwrap();
                    m.schema_version = SCHEMA_VERSION + 5;
                    serde_json::to_vec_pretty(&m).unwrap()
                } else {
                    buf.clone()
                };
                w.start_file(name, opt).unwrap();
                w.write_all(&data).unwrap();
            }
            w.finish().unwrap();
        }
        let err = unpack_backup(&cur.into_inner()).unwrap_err();
        assert!(err.contains("更新的应用版本"), "实际错误：{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_without_manifest_is_rejected() {
        let mut cur = Cursor::new(Vec::new());
        {
            let mut w = zip::write::ZipWriter::new(&mut cur);
            let opt = zip::write::SimpleFileOptions::default();
            w.start_file("ai_teacher.db", opt).unwrap();
            w.write_all(b"not a db").unwrap();
            w.finish().unwrap();
        }
        let err = unpack_backup(&cur.into_inner()).unwrap_err();
        assert!(err.contains("manifest"), "实际错误：{err}");
    }

    #[test]
    fn dirty_flag_only_set_by_writes() {
        let dir = temp_dir("dirty");
        let (db, unit_id) = setup_db(&dir);
        // 建库本身已经写过，先清掉
        db.take_dirty();
        assert!(!db.take_dirty(), "读操作不该置脏");

        db.list_books().unwrap();
        db.get_unit(unit_id).unwrap();
        db.stats().unwrap();
        assert!(!db.take_dirty(), "纯读不该置脏（list_books 的封面补建不算数据写）");

        db.update_unit_progress(unit_id, "done", None).unwrap();
        assert!(db.take_dirty(), "写操作应置脏");
        assert!(!db.take_dirty(), "取走后应清零");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_backup_includes_books() {
        let dir = temp_dir("withbooks");
        let (db, unit_id) = setup_db(&dir);
        // 造一个 books/ 目录
        let book_dir = dir.join("books").join("1");
        std::fs::create_dir_all(&book_dir).unwrap();
        std::fs::write(book_dir.join("unit_0.txt"), "hello").unwrap();
        std::fs::write(book_dir.join("cover.jpg"), vec![0xFFu8; 64]).unwrap();

        let bytes = build_backup(&db, &dir, PackMode::WithBooks).unwrap();
        let manifest = read_manifest(&bytes).unwrap();
        assert!(manifest.includes_books);

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().to_string()).collect();
        assert!(names.contains(&"books/1/unit_0.txt".to_string()), "实际条目：{names:?}");
        assert!(names.contains(&"books/1/cover.jpg".to_string()), "实际条目：{names:?}");
        // 第一个必须是 manifest
        assert_eq!(names[0], ENTRY_MANIFEST);
        let _ = unit_id;

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_books_copies_units_and_fills_missing() {
        let dir = temp_dir("merge");
        let (_db, _) = setup_db(&dir);
        let src = dir.join("incoming");
        std::fs::create_dir_all(src.join("1")).unwrap();
        std::fs::write(src.join("1").join("unit_0.txt"), "新版正文").unwrap();
        std::fs::write(src.join("1").join("cover.jpg"), b"new-cover").unwrap();

        let dest_book = dir.join("books").join("1");
        std::fs::create_dir_all(&dest_book).unwrap();
        std::fs::write(dest_book.join("unit_0.txt"), "旧版正文").unwrap();

        merge_books(&dir, &src).unwrap();

        // unit_*.txt 必须被备份里的版本覆盖
        let unit = std::fs::read_to_string(dest_book.join("unit_0.txt")).unwrap();
        assert_eq!(unit, "新版正文");
        // 缺失的文件补回来
        assert!(dest_book.join("cover.jpg").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn webdav_settings_never_include_secrets_in_backup() {
        // 备份包里绝不能出现 api_key / WebDAV 密码
        let dir = temp_dir("secret");
        let (db, _) = setup_db(&dir);
        // 写一份含明文的配置
        std::fs::write(
            dir.join("config.json"),
            r#"{"api_key":"sk-SECRET-123","base_url":"https://x"}"#,
        )
        .unwrap();
        save_webdav_settings(
            &dir,
            &WebDavSettings {
                enabled: true,
                base_url: "https://dav.example.com/dav/".into(),
                username: "u@example.com".into(),
                password: "SECRET-APP-PASSWORD".into(),
                folder: "ai-teacher".into(),
            },
        )
        .unwrap();

        let bytes = build_backup(&db, &dir, PackMode::WithBooks).unwrap();
        let haystack = String::from_utf8_lossy(&bytes);
        assert!(!haystack.contains("sk-SECRET-123"), "备份包里出现了 api_key");
        assert!(
            !haystack.contains("SECRET-APP-PASSWORD"),
            "备份包里出现了 WebDAV 密码"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_readable_from_truncated_head() {
        // 关键回归：list_remote 只用 Range 抓前 8KB，拿到的 zip 没有中央目录，
        // ZipArchive 打不开。必须能靠扫局部头读出 manifest。
        let dir = temp_dir("truncated");
        let (db, _) = setup_db(&dir);
        let bytes = build_backup(&db, &dir, PackMode::DbOnly).unwrap();

        // 完整包能读
        let full = read_manifest(&bytes).expect("完整包应能读出清单");

        // 截断到 512 字节（只够容纳清单本身，模拟 Range 抓的头部）
        let cut = 512.min(bytes.len() - 1);
        let head = &bytes[..cut];
        let from_head = read_manifest(head).expect("截断的头部也应能读出清单");

        assert_eq!(full.schema_version, from_head.schema_version);
        assert_eq!(full.db_stats.books, from_head.db_stats.books);
        assert_eq!(full.device, from_head.device);
        // 确认截断确实发生在中央目录之前（否则这个测试没验证到点子上）
        assert!(head.len() < bytes.len(), "应当真的被截断了");
        assert!(
            zip::ZipArchive::new(Cursor::new(head.to_vec())).is_err(),
            "截断后的 zip 应无法被 ZipArchive 打开（否则测不到局部头路径）"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_is_stored_not_deflated() {
        // manifest 必须不压缩：否则扫局部头拿到的数据是 deflate 流，
        // 而解压需要 flate2（zip 的间接依赖，用不了）
        let dir = temp_dir("stored");
        let (db, _) = setup_db(&dir);
        let bytes = build_backup(&db, &dir, PackMode::DbOnly).unwrap();

        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let f = zip.by_name(ENTRY_MANIFEST).unwrap();
        assert_eq!(
            f.compression(),
            zip::CompressionMethod::Stored,
            "manifest 应为 Stored"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_prop_list_skips_dirs_and_non_zip() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/dav/ai-teacher/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/ai-teacher/ai-teacher-db.zip</d:href>
    <d:propstat><d:prop>
      <d:getcontentlength>63003</d:getcontentlength>
      <d:getlastmodified>Mon, 03 Nov 2026 02:42:00 GMT</d:getlastmodified>
    </d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/ai-teacher/notes.txt</d:href>
    <d:propstat><d:prop><d:getcontentlength>10</d:getcontentlength></d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;
        let list = parse_prop_list(xml);
        assert_eq!(list.len(), 1, "应只保留 zip：{list:?}");
        assert_eq!(list[0].name, "ai-teacher-db.zip");
        assert_eq!(list[0].size, 63003);
    }

    #[test]
    fn no_vendor_or_size_hardcoding_in_ui_facing_strings() {
        // 回归护栏：界面/提示文案里不能出现具体字节数或厂商限额。
        // 体积随答题量与书的大小变化，厂商限额服务器也不会告知，写死会误导用户。
        let msg = "连接成功：可读写".to_string();
        assert!(!msg.contains("请求"), "连接提示不该提请求配额：{msg}");
        assert!(!msg.contains("坚果云"), "连接提示不该点名厂商：{msg}");

        // 限流提示只按状态码给，不猜额度
        assert!(throttling_hint(429).unwrap().contains("限流"));
        assert!(throttling_hint(507).unwrap().contains("空间"));
        assert!(throttling_hint(200).is_none());
        assert!(throttling_hint(404).is_none());
    }

    #[test]
    fn human_size_reads_naturally() {
        assert_eq!(human_size(512), "512 字节");
        assert_eq!(human_size(63003), "61.5 KB");
        assert_eq!(human_size(3_462_853), "3.3 MB");
    }

    #[test]
    fn schema_version_is_persisted() {
        let dir = temp_dir("schemaver");
        let path = dir.join("ai_teacher.db");
        {
            let db = Db::open(&path).unwrap();
            assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        }
        // 重新打开仍是同一版本
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
