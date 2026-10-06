use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use crate::db::Db;
use crate::models::BookSummary;

/// 一个章节：标题 + 段落列表。
#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub paragraphs: Vec<String>,
}

/// 切分后的学习单元。
#[derive(Debug, Clone)]
pub struct UnitDraft {
    pub title: String,
    pub text: String,
}

const MIN_UNIT_CHARS: usize = 1500;
const MAX_UNIT_CHARS: usize = 3000;
const MIN_TAIL_CHARS: usize = 700;

// ---------- 文件入口 ----------

pub fn parse_file(path: &Path) -> Result<Vec<Chapter>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => Ok(parse_markdown(&read_text(path)?)),
        "txt" => Ok(parse_txt(&read_text(path)?)),
        "epub" => {
            let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件：{e}"))?;
            parse_epub(&bytes)
        }
        "pdf" => parse_pdf(path),
        _ => Err("暂不支持该文件格式，仅支持 md / txt / epub / pdf".to_string()),
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("无法读取文件：{e}"))
}

// ---------- Markdown ----------

pub fn parse_markdown(text: &str) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut title: Option<String> = None;
    let mut paragraphs: Vec<String> = Vec::new();
    let mut lines: Vec<&str> = Vec::new();

    fn flush_paragraph(paragraphs: &mut Vec<String>, lines: &mut Vec<&str>) {
        if !lines.is_empty() {
            let joined = lines.join("\n").trim().to_string();
            if !joined.is_empty() {
                paragraphs.push(joined);
            }
            lines.clear();
        }
    }

    fn push_chapter(
        chapters: &mut Vec<Chapter>,
        title: &mut Option<String>,
        paragraphs: &mut Vec<String>,
    ) {
        let paras = std::mem::take(paragraphs);
        if paras.is_empty() {
            return;
        }
        let chapter_title = match title.take() {
            Some(t) => t,
            None => "前言".to_string(),
        };
        chapters.push(Chapter {
            title: chapter_title,
            paragraphs: paras,
        });
    }

    for line in text.lines() {
        if line.starts_with("# ") {
            flush_paragraph(&mut paragraphs, &mut lines);
            push_chapter(&mut chapters, &mut title, &mut paragraphs);
            title = Some(line[2..].trim().to_string());
        } else if line.trim().is_empty() {
            flush_paragraph(&mut paragraphs, &mut lines);
        } else {
            lines.push(line.trim_end());
        }
    }
    flush_paragraph(&mut paragraphs, &mut lines);
    push_chapter(&mut chapters, &mut title, &mut paragraphs);
    chapters
}

// ---------- 纯文本 ----------

pub fn parse_txt(text: &str) -> Vec<Chapter> {
    let paragraphs = split_paragraphs(text);
    if paragraphs.is_empty() {
        Vec::new()
    } else {
        vec![Chapter {
            title: "正文".to_string(),
            paragraphs,
        }]
    }
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n").trim().to_string());
                current.clear();
            }
        } else {
            current.push(line.trim_end());
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n").trim().to_string());
    }
    paragraphs.into_iter().filter(|p| !p.is_empty()).collect()
}

// ---------- PDF ----------

fn parse_pdf(path: &Path) -> Result<Vec<Chapter>, String> {
    let text = pdf_extract::extract_text(path).map_err(|e| format!("无法解析 PDF：{e}"))?;
    // 换页符当作段落分隔
    let text = text.replace('\u{c}', "\n\n");
    let paragraphs = split_paragraphs(&text);
    if paragraphs.is_empty() {
        return Err("PDF 中没有可提取的文本内容".to_string());
    }
    Ok(vec![Chapter {
        title: "正文".to_string(),
        paragraphs,
    }])
}

// ---------- EPUB ----------

pub fn parse_epub(bytes: &[u8]) -> Result<Vec<Chapter>, String> {
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("无法读取 EPUB 文件：{e}"))?;
    let container = read_zip_text(&mut zip, "META-INF/container.xml")
        .ok_or("EPUB 缺少 META-INF/container.xml")?;
    let opf_path = parse_container(&container).ok_or("EPUB 缺少 OPF 路径信息")?;
    let opf_dir = parent_dir(&opf_path);
    let opf = read_zip_text(&mut zip, &opf_path).ok_or_else(|| format!("EPUB 缺少 {opf_path}"))?;
    let opf_data = parse_opf(&opf);

    let mut toc: HashMap<String, String> = HashMap::new();
    if let Some(toc_id) = &opf_data.toc_id {
        if let Some(href) = opf_data.manifest.get(toc_id) {
            let path = resolve_href(&opf_dir, href);
            if let Some(ncx) = read_zip_text(&mut zip, &path) {
                parse_ncx(&ncx, &opf_dir, &mut toc);
            }
        }
    }
    if let Some(href) = &opf_data.nav_href {
        let path = resolve_href(&opf_dir, href);
        if let Some(nav) = read_zip_text(&mut zip, &path) {
            parse_nav(&nav, &opf_dir, &mut toc);
        }
    }

    let mut chapters = Vec::new();
    for (i, idref) in opf_data.spine.iter().enumerate() {
        let Some(href) = opf_data.manifest.get(idref) else {
            continue;
        };
        let path = resolve_href(&opf_dir, href);
        let Some(xhtml) = read_zip_text(&mut zip, &path) else {
            continue;
        };
        let (paragraphs, doc_title) = xhtml_to_paragraphs(&xhtml);
        if paragraphs.is_empty() {
            continue;
        }
        let title = toc
            .get(&path)
            .cloned()
            .or_else(|| lookup_by_basename(&toc, &path))
            .or(doc_title)
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        chapters.push(Chapter { title, paragraphs });
    }
    if chapters.is_empty() {
        return Err("EPUB 中没有可读的正文内容".to_string());
    }
    Ok(chapters)
}

fn read_zip_text<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Option<String> {
    for candidate in zip_name_candidates(name) {
        if let Ok(mut file) = zip.by_name(&candidate) {
            let mut buf = String::new();
            if file.read_to_string(&mut buf).is_ok() && !buf.is_empty() {
                return Some(buf);
            }
        }
    }
    None
}

fn zip_name_candidates(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    let normalized = name.replace('\\', "/");
    if !out.contains(&normalized) {
        out.push(normalized.clone());
    }
    let stripped = normalized.trim_start_matches("./").to_string();
    if !out.contains(&stripped) {
        out.push(stripped);
    }
    out
}

fn parse_container(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"rootfile" {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"full-path" {
                            if let Ok(v) = attr.unescape_value() {
                                return Some(v.into_owned());
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        buf.clear();
    }
}

struct OpfData {
    manifest: HashMap<String, String>,
    spine: Vec<String>,
    toc_id: Option<String>,
    nav_href: Option<String>,
}

fn parse_opf(xml: &str) -> OpfData {
    let mut data = OpfData {
        manifest: HashMap::new(),
        spine: Vec::new(),
        toc_id: None,
        nav_href: None,
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut section = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"manifest" => section = "manifest".to_string(),
                b"spine" => {
                    section = "spine".to_string();
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"toc" {
                            if let Ok(v) = attr.unescape_value() {
                                data.toc_id = Some(v.into_owned());
                            }
                        }
                    }
                }
                b"item" if section == "manifest" => collect_manifest_item(&e, &mut data),
                b"itemref" if section == "spine" => {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"idref" {
                            if let Ok(v) = attr.unescape_value() {
                                data.spine.push(v.into_owned());
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"item" if section == "manifest" => collect_manifest_item(&e, &mut data),
                b"itemref" if section == "spine" => {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"idref" {
                            if let Ok(v) = attr.unescape_value() {
                                data.spine.push(v.into_owned());
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"manifest" | b"spine" => section.clear(),
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }
    data
}

fn collect_manifest_item(e: &quick_xml::events::BytesStart<'_>, data: &mut OpfData) {
    let mut id = None;
    let mut href = None;
    let mut properties = String::new();
    for attr in e.attributes().flatten() {
        match local_name(attr.key.as_ref()) {
            b"id" => id = attr.unescape_value().ok().map(|v| v.into_owned()),
            b"href" => href = attr.unescape_value().ok().map(|v| v.into_owned()),
            b"properties" => {
                properties = attr
                    .unescape_value()
                    .map(|v| v.into_owned())
                    .unwrap_or_default()
            }
            _ => {}
        }
    }
    if let (Some(id), Some(href)) = (id, href) {
        if properties
            .split_whitespace()
            .any(|p| p.eq_ignore_ascii_case("nav"))
        {
            data.nav_href = Some(href.clone());
        }
        data.manifest.insert(id, href);
    }
}

fn parse_ncx(xml: &str, opf_dir: &str, toc: &mut HashMap<String, String>) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut current_label: Option<String> = None;
    let mut in_label_text = false;
    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"text" {
                    in_label_text = true;
                }
            }
            Ok(Event::Empty(e)) => {
                // <content src="..."/> 是自闭合标签
                if local_name(e.name().as_ref()) == b"content" {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"src" {
                            if let Ok(v) = attr.unescape_value() {
                                if let Some(label) = &current_label {
                                    if !label.trim().is_empty() {
                                        toc.insert(
                                            resolve_href(opf_dir, &v),
                                            label.trim().to_string(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if in_label_text {
                    current_label = t.unescape().ok().map(|v| v.into_owned());
                    in_label_text = false;
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == b"text" {
                    in_label_text = false;
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }
}

fn parse_nav(xml: &str, opf_dir: &str, toc: &mut HashMap<String, String>) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut pending_href: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"a" {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"href" {
                            pending_href = attr.unescape_value().ok().map(|v| v.into_owned());
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(href) = pending_href.take() {
                    let title = t.unescape().unwrap_or_default();
                    if !title.trim().is_empty() {
                        toc.insert(resolve_href(opf_dir, &href), title.trim().to_string());
                    }
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == b"a" {
                    pending_href = None;
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }
}

/// 把 XHTML 去标签成段落，同时顺带取 <title>。
fn xhtml_to_paragraphs(html: &str) -> (Vec<String>, Option<String>) {
    let mut reader = Reader::from_str(html);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut title = String::new();
    let mut capture_title = false;
    let mut skip_depth = 0i32;
    let mut in_head = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"style" | b"script" => skip_depth += 1,
                b"head" => in_head = true,
                b"title" => capture_title = true,
                b"body" => in_head = false,
                name if is_block_tag(name) => flush_paragraph(&mut paragraphs, &mut current),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if is_block_tag(local_name(e.name().as_ref())) {
                    flush_paragraph(&mut paragraphs, &mut current);
                }
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"style" | b"script" => skip_depth -= 1,
                b"head" => in_head = false,
                b"title" => capture_title = false,
                name if is_block_tag(name) => flush_paragraph(&mut paragraphs, &mut current),
                _ => {}
            },
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default();
                if !text.trim().is_empty() {
                    if capture_title {
                        title.push_str(&text);
                    } else if skip_depth == 0 && !in_head {
                        if !current.is_empty() {
                            current.push(' ');
                        }
                        current.push_str(&text);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }
    flush_paragraph(&mut paragraphs, &mut current);
    let title = title.trim();
    (
        paragraphs,
        if title.is_empty() {
            None
        } else {
            Some(title.to_string())
        },
    )
}

fn flush_paragraph(paragraphs: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        paragraphs.push(trimmed.to_string());
    }
    current.clear();
}

fn is_block_tag(name: &[u8]) -> bool {
    matches!(
        name,
        b"p" | b"div"
            | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"li"
            | b"br"
            | b"ul"
            | b"ol"
            | b"table"
            | b"tr"
            | b"td"
            | b"th"
            | b"section"
            | b"article"
            | b"blockquote"
            | b"pre"
            | b"hr"
            | b"figure"
            | b"figcaption"
            | b"header"
            | b"footer"
            | b"aside"
    )
}

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&c| c == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

fn lookup_by_basename(toc: &HashMap<String, String>, path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path);
    toc.iter()
        .find(|(k, _)| k.rsplit('/').next() == Some(base))
        .map(|(_, v)| v.clone())
}

fn resolve_href(base_dir: &str, href: &str) -> String {
    let without_fragment = href.split('#').next().unwrap_or(href);
    let decoded = percent_decode(without_fragment).replace('\\', "/");
    let joined = if decoded.starts_with('/') || base_dir.is_empty() {
        decoded
    } else {
        format!("{base_dir}/{decoded}")
    };
    normalize_zip_path(&joined)
}

fn normalize_zip_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------- 章节切分为学习单元 ----------

pub fn chunk_chapters(chapters: Vec<Chapter>) -> Vec<UnitDraft> {
    let mut units = Vec::new();
    for chapter in chapters {
        let pieces = chunk_paragraphs(&chapter.paragraphs);
        let count = pieces.len();
        for (i, text) in pieces.into_iter().enumerate() {
            let title = if count > 1 {
                format!("{}（{}）", chapter.title, i + 1)
            } else {
                chapter.title.clone()
            };
            units.push(UnitDraft { title, text });
        }
    }
    units
}

/// 段落打包为 1500–3000 字的单元；末尾不足 700 字的块并入前一单元。
pub fn chunk_paragraphs(paragraphs: &[String]) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for p in paragraphs {
        if p.chars().count() > MAX_UNIT_CHARS {
            pieces.extend(split_long_paragraph(p));
        } else if !p.trim().is_empty() {
            pieces.push(p.clone());
        }
    }

    let mut units: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for piece in pieces {
        let plen = piece.chars().count();
        if current_len > 0 && current_len + plen > MAX_UNIT_CHARS {
            units.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(&piece);
        current_len += plen;
        if current_len >= MIN_UNIT_CHARS {
            units.push(std::mem::take(&mut current));
            current_len = 0;
        }
    }
    if current_len > 0 {
        units.push(current);
    }

    if units.len() >= 2 {
        let last = units.pop().unwrap();
        if last.chars().count() < MIN_TAIL_CHARS {
            if let Some(prev) = units.last_mut() {
                prev.push('\n');
                prev.push_str(&last);
            }
        } else {
            units.push(last);
        }
    }
    units.into_iter().map(|u| u.trim().to_string()).collect()
}

/// 超长段落在句边界（。！？!?.\n）处切分，尽量贴近 3000 字。
fn split_long_paragraph(p: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    for sentence in split_sentences(p) {
        let slen = sentence.chars().count();
        if slen > MAX_UNIT_CHARS {
            if !current.trim().is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
            pieces.extend(hard_chop(&sentence));
            continue;
        }
        if !current.is_empty() && current.chars().count() + slen > MAX_UNIT_CHARS {
            pieces.push(std::mem::take(&mut current));
        }
        current.push_str(&sentence);
    }
    if !current.trim().is_empty() {
        pieces.push(current);
    }
    pieces
}

fn split_sentences(p: &str) -> Vec<String> {
    let chars: Vec<char> = p.chars().collect();
    let mut sentences = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        current.push(c);
        if matches!(c, '。' | '！' | '？' | '!' | '?' | '.' | '\n') {
            // 不在小数点中间断句（如 3.14）
            let next_is_digit = c == '.' && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            if !next_is_digit {
                sentences.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current);
    }
    sentences
}

fn hard_chop(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in s.chars() {
        current.push(c);
        if current.chars().count() >= MAX_UNIT_CHARS {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

// ---------- 导入 ----------

pub fn import_book(db: &Db, app_data_dir: &Path, path: &str) -> Result<BookSummary, String> {
    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(format!("文件不存在：{path}"));
    }
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let format = match ext.as_str() {
        "md" | "markdown" => "md",
        "txt" => "txt",
        "epub" => "epub",
        "pdf" => "pdf",
        _ => return Err("暂不支持该文件格式，仅支持 md / txt / epub / pdf".to_string()),
    };
    if db
        .find_book_by_source_path(path)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("这本书已经导入过了".to_string());
    }

    let title = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未命名".to_string());
    let chapters = parse_file(file_path)?;
    let units = chunk_chapters(chapters);
    if units.is_empty() {
        return Err("未能从该文件中解析出任何学习内容".to_string());
    }

    let added_at = chrono::Local::now().format("%Y-%m-%d").to_string();
    let book_id = db
        .insert_book(&title, path, format, &added_at)
        .map_err(|e| e.to_string())?;
    let book_dir = app_data_dir.join("books").join(book_id.to_string());
    if let Err(e) = persist_units(db, book_id, &book_dir, &units) {
        let _ = db.delete_book(book_id);
        let _ = std::fs::remove_dir_all(&book_dir);
        return Err(e);
    }

    Ok(BookSummary {
        id: book_id,
        title,
        format: format.to_string(),
        added_at,
        total_units: units.len() as i64,
        done_units: 0,
    })
}

fn persist_units(
    db: &Db,
    book_id: i64,
    book_dir: &Path,
    units: &[UnitDraft],
) -> Result<(), String> {
    std::fs::create_dir_all(book_dir).map_err(|e| format!("无法创建书籍目录：{e}"))?;
    for (idx, unit) in units.iter().enumerate() {
        let file = book_dir.join(format!("unit_{idx}.txt"));
        std::fs::write(&file, &unit.text).map_err(|e| format!("无法写入单元文件：{e}"))?;
        db.insert_unit(book_id, idx as i64, &unit.title, &file.to_string_lossy())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::{SimpleFileOptions, ZipWriter};

    fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/demo-book.md")
    }

    fn s(n: usize) -> String {
        "字".repeat(n)
    }

    fn content_len(unit: &str) -> usize {
        unit.chars().filter(|c| !c.is_whitespace()).count()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ai-teacher-ingest-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_demo_book_markdown() {
        let text = std::fs::read_to_string(fixture_path()).unwrap();
        let chapters = parse_markdown(&text);
        assert_eq!(chapters.len(), 3);
        assert!(chapters[0].title.contains("第一章"));
        assert!(chapters[1].title.contains("第二章"));
        assert!(chapters[2].title.contains("第三章"));
        // demo 书每章都不足 1500 字，一章一个单元
        let units = chunk_chapters(chapters);
        assert_eq!(units.len(), 3);
        assert!(units[0].title.contains("第一章"));
        assert!(units[0].text.contains("编码"));
        assert!(units[1].text.contains("间隔效应"));
        assert!(units[2].text.contains("提取练习"));
        for unit in &units {
            assert!(unit.text.chars().count() > 100);
        }
    }

    #[test]
    fn markdown_preamble_becomes_preface() {
        let md = "开场白段落。\n\n# 第一章 A\n\n内容一。\n\n# 第二章 B\n\n内容二。\n";
        let chapters = parse_markdown(md);
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "前言");
        assert_eq!(chapters[0].paragraphs, vec!["开场白段落。".to_string()]);
        assert_eq!(chapters[1].title, "第一章 A");
        // 空章节被跳过
        let empty_heading = "# 有标题\n\n# 下一个\n\n内容。\n";
        let chapters = parse_markdown(empty_heading);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "下一个");
    }

    #[test]
    fn chunk_packing_and_tail_merge() {
        // 短尾并入前一个单元
        let units = chunk_paragraphs(&[s(1600), s(200)]);
        assert_eq!(units.len(), 1);
        assert_eq!(content_len(&units[0]), 1800);

        // 800 + 800 先打包满 1500 关闭，尾 400 并入
        let units = chunk_paragraphs(&[s(800), s(800), s(400)]);
        assert_eq!(units.len(), 1);
        assert_eq!(content_len(&units[0]), 2000);

        // 每段超过 1500，各自成单元，尾段不合并
        let units = chunk_paragraphs(&[s(1600), s(1600), s(1600)]);
        assert_eq!(units.len(), 3);

        // 1400 + 1400 打包成一个 2800 的单元
        let units = chunk_paragraphs(&[s(1400), s(1400)]);
        assert_eq!(units.len(), 1);
        assert_eq!(content_len(&units[0]), 2800);

        // 2000 + 500：500 < 700 并入 → 2500
        let units = chunk_paragraphs(&[s(2000), s(500)]);
        assert_eq!(units.len(), 1);
        assert_eq!(content_len(&units[0]), 2500);
    }

    #[test]
    fn long_paragraph_splits_on_sentence_boundaries() {
        let sentence = format!("{}。", "字".repeat(99));
        let para = sentence.repeat(70);
        assert_eq!(para.chars().count(), 7000);
        let units = chunk_paragraphs(&[para]);
        assert_eq!(units.len(), 3);
        assert_eq!(content_len(&units[0]), 3000);
        assert_eq!(content_len(&units[1]), 3000);
        assert_eq!(content_len(&units[2]), 1000);
        for unit in &units {
            assert!(unit.ends_with('。'));
        }
    }

    #[test]
    fn split_long_paragraph_hard_chops_boundaryless_text() {
        let para = format!("{}。{}", "甲".repeat(3100), "乙".repeat(50));
        let pieces = split_long_paragraph(&para);
        assert!(pieces.len() >= 2);
        assert!(pieces.iter().all(|p| p.chars().count() <= MAX_UNIT_CHARS));
        assert_eq!(pieces[0].chars().count(), MAX_UNIT_CHARS);
        let total: usize = pieces.iter().map(|p| p.chars().count()).sum();
        assert_eq!(total, para.chars().count());
    }

    #[test]
    fn sentence_split_avoids_decimal_points() {
        let sentences = split_sentences("圆周率是 3.14 它很重要。再见！");
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0], "圆周率是 3.14 它很重要。");
    }

    #[test]
    fn multi_unit_chapter_gets_numbered_titles() {
        let chapters = vec![Chapter {
            title: "长章".to_string(),
            paragraphs: vec![s(1600), s(1600)],
        }];
        let units = chunk_chapters(chapters);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].title, "长章（1）");
        assert_eq!(units[1].title, "长章（2）");
    }

    #[test]
    fn parse_txt_paragraphs() {
        let text = "第一段第一行\n第一段第二行\n\n第二段。\n\n\n第三段。\n";
        let chapters = parse_txt(text);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "正文");
        assert_eq!(chapters[0].paragraphs.len(), 3);
        assert!(chapters[0].paragraphs[0].contains("第一段第二行"));
        assert_eq!(chapters[0].paragraphs[1], "第二段。");
    }

    #[test]
    fn href_resolution() {
        assert_eq!(
            resolve_href("OEBPS", "text/a1.xhtml"),
            "OEBPS/text/a1.xhtml"
        );
        assert_eq!(
            resolve_href("OEBPS", "text/a1.xhtml#sec1"),
            "OEBPS/text/a1.xhtml"
        );
        assert_eq!(resolve_href("OEBPS", "../cover.xhtml"), "cover.xhtml");
        assert_eq!(resolve_href("", "cover.xhtml"), "cover.xhtml");
        assert_eq!(
            resolve_href("OEBPS", "my%20file.xhtml"),
            "OEBPS/my file.xhtml"
        );
        assert_eq!(resolve_href("OEBPS", "/root.xhtml"), "root.xhtml");
        assert_eq!(
            resolve_href("OEBPS", "./text/a1.xhtml"),
            "OEBPS/text/a1.xhtml"
        );
    }

    fn write_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut buf);
            let options = SimpleFileOptions::default();
            for (name, content) in files {
                writer.start_file(*name, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buf.into_inner()
    }

    const CONTAINER_XML: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const OPF_WITH_NCX: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata><dc:title>测试书</dc:title></metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="c1" href="text/a1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/a2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>"#;

    const NCX: &str = r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <docTitle><text>测试书</text></docTitle>
  <navMap>
    <navPoint id="n1"><navLabel><text>第一章 起点</text></navLabel><content src="text/a1.xhtml"/></navPoint>
    <navPoint id="n2"><navLabel><text>第二章 转折</text></navLabel><content src="text/a2.xhtml"/></navPoint>
  </navMap>
</ncx>"#;

    fn xhtml(title: &str, paras: &[&str]) -> String {
        let body: String = paras
            .iter()
            .map(|p| format!("<p>{p}</p>"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>{title}</title><style type="text/css">p {{ color: red; }}</style></head>
<body>
{body}
</body>
</html>"#
        )
    }

    #[test]
    fn parse_minimal_epub_with_ncx_titles() {
        let bytes = write_zip(&[
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", OPF_WITH_NCX),
            ("OEBPS/toc.ncx", NCX),
            (
                "OEBPS/text/a1.xhtml",
                &xhtml(
                    "文档标题一",
                    &["第一章的正文第一段，讲述起点。", "第一章的正文第二段。"],
                ),
            ),
            (
                "OEBPS/text/a2.xhtml",
                &xhtml("文档标题二", &["第二章的正文。"]),
            ),
        ]);
        let chapters = parse_epub(&bytes).unwrap();
        assert_eq!(chapters.len(), 2);
        // 目录（NCX）优先于文档 title
        assert_eq!(chapters[0].title, "第一章 起点");
        assert_eq!(chapters[1].title, "第二章 转折");
        assert_eq!(chapters[0].paragraphs.len(), 2);
        assert!(chapters[0].paragraphs[0].contains("讲述起点"));
        // style 内容不进入正文
        assert!(!chapters[0].paragraphs.iter().any(|p| p.contains("color")));
    }

    #[test]
    fn parse_epub_falls_back_to_doc_title() {
        let opf_no_toc = OPF_WITH_NCX.replace(" toc=\"ncx\"", "");
        let bytes = write_zip(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", &opf_no_toc),
            ("OEBPS/text/a1.xhtml", &xhtml("甲卷", &["甲卷的内容。"])),
        ]);
        let chapters = parse_epub(&bytes).unwrap();
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "甲卷");
        assert!(chapters[0].paragraphs[0].contains("甲卷的内容"));
    }

    #[test]
    fn parse_epub_rejects_bad_files() {
        assert!(parse_epub(b"not a zip").is_err());
        let bytes = write_zip(&[("unrelated.txt", "hello")]);
        assert!(parse_epub(&bytes).is_err());
    }

    #[test]
    fn import_book_end_to_end() {
        let dir = temp_dir("import");
        let db = Db::open_in_memory().unwrap();
        let path = fixture_path().to_string_lossy().to_string();
        let summary = import_book(&db, &dir, &path).unwrap();
        assert_eq!(summary.title, "demo-book");
        assert_eq!(summary.format, "md");
        assert_eq!(summary.total_units, 3);
        assert_eq!(summary.done_units, 0);

        let book_dir = dir.join("books").join(summary.id.to_string());
        assert!(book_dir.join("unit_0.txt").exists());
        assert!(book_dir.join("unit_2.txt").exists());

        let units = db.list_units(summary.id).unwrap();
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].idx, 0);
        assert!(units[0].title.contains("第一章"));
        let content = std::fs::read_to_string(book_dir.join("unit_1.txt")).unwrap();
        assert!(content.contains("间隔效应"));

        // 重复导入被拒绝
        let err = import_book(&db, &dir, &path).unwrap_err();
        assert_eq!(err, "这本书已经导入过了");

        // 不支持的格式
        let txt = dir.join("sample.xyz");
        std::fs::write(&txt, "x").unwrap();
        let err = import_book(&db, &dir, txt.to_string_lossy().as_ref()).unwrap_err();
        assert!(err.contains("暂不支持"));
        // 不存在的文件
        let err = import_book(&db, &dir, "Z:/不存在.md").unwrap_err();
        assert!(err.contains("文件不存在"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
