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

/// 解析结果：章列表 + 每章所属"大节"标题（EPUB 目录层级的顶层条目；无层级时为章自身标题）
/// + 抽取出的图片（输出文件名，字节内容，导入时写入 books/<id>/images/）。
#[derive(Debug, Clone)]
pub struct ParsedBook {
    pub chapters: Vec<Chapter>,
    pub group_titles: Vec<String>,
    pub images: Vec<(String, Vec<u8>)>,
}

impl ParsedBook {
    /// 无目录结构的格式（md/txt/pdf）：每章即大节。
    fn flat(chapters: Vec<Chapter>) -> Self {
        let group_titles = chapters.iter().map(|c| c.title.clone()).collect();
        ParsedBook {
            chapters,
            group_titles,
            images: Vec::new(),
        }
    }
}

/// 解析书籍文件，EPUB 保留目录层级（大节归属），其余格式每章即大节。
pub fn parse_file_grouped(path: &Path) -> Result<ParsedBook, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => Ok(ParsedBook::flat(parse_markdown(&read_text(path)?))),
        "txt" => Ok(ParsedBook::flat(parse_txt(&read_text(path)?))),
        "epub" => {
            let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件：{e}"))?;
            parse_epub_grouped(&bytes)
        }
        "pdf" => parse_pdf(path).map(ParsedBook::flat),
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

/// 页眉页脚判定：同一行文本出现在超过该比例的页里即视为重复装饰。
const RUNNING_HEADER_RATIO: f64 = 0.5;
/// 认定为页眉页脚的行位置（从页首/页尾数起的前/后若干行）。
const EDGE_LINES: usize = 3;
/// 标题行最大长度（字符），超过视为正文。
const MAX_HEADING_CHARS: usize = 40;

fn parse_pdf(path: &Path) -> Result<Vec<Chapter>, String> {
    let pages = extract_pdf_pages(path)?;
    let pages = strip_running_headers(&pages);
    let blocks = pages
        .iter()
        .flat_map(|page| pdf_page_paragraphs(page))
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return Err("PDF 中没有可提取的文本内容（可能是扫描件或纯图片 PDF）".to_string());
    }
    let chapters = split_pdf_chapters(blocks);
    if chapters.is_empty() {
        return Err("PDF 中没有可提取的文本内容（可能是扫描件或纯图片 PDF）".to_string());
    }
    Ok(chapters)
}

/// 按页提取文本：先走 pdf-extract（版式还原更好）；它对 GBK-EUC-H 等预定义
/// CMap 编码直接 assert 崩溃且不含文字时，回退 lopdf（支持编码更广但按原文
/// 绘制顺序逐字输出，需要再合并单字行）。
fn extract_pdf_pages(path: &Path) -> Result<Vec<String>, String> {
    match extract_with_pdf_extract(path) {
        Ok(pages) if pages.iter().any(|p| !p.trim().is_empty()) => Ok(pages),
        Ok(_) | Err(_) => extract_with_lopdf(path),
    }
}

fn extract_with_pdf_extract(path: &Path) -> Result<Vec<String>, String> {
    // 静音这段时间的 panic 输出：库内部的 assert 对用户只是噪音，错误由下方统一提示。
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_by_pages(path)
    }));
    std::panic::set_hook(previous_hook);
    match result {
        Ok(Ok(pages)) => Ok(pages),
        Ok(Err(e)) => Err(format!("无法解析 PDF：{e}")),
        Err(_) => Err("无法解析 PDF：该文件使用了暂不支持的字体编码".to_string()),
    }
}

fn extract_with_lopdf(path: &Path) -> Result<Vec<String>, String> {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let doc = lopdf::Document::load(path).map_err(|e| format!("无法解析 PDF：{e}"))?;
        let mut pages: Vec<u32> = doc.get_pages().keys().cloned().collect();
        pages.sort_unstable();
        pages
            .iter()
            .map(|n| Ok(doc.extract_text(&[*n]).unwrap_or_default()))
            .collect::<Result<Vec<_>, String>>()
    }));
    std::panic::set_hook(previous_hook);
    match result {
        Ok(Ok(pages)) if pages.iter().any(|p| !p.trim().is_empty()) => {
            Ok(pages.iter().map(|p| merge_glyph_runs(p)).collect())
        }
        Ok(Ok(_)) => Err("PDF 中没有可提取的文本内容（可能是扫描件或纯图片 PDF）".to_string()),
        Ok(Err(e)) => Err(format!(
            "{e}。可尝试用阅读器另存为 PDF 或先转成 EPUB/TXT 后再导入"
        )),
        Err(_) => Err(
            "无法解析 PDF：该文件可能已损坏或使用了暂不支持的结构。\
             可尝试用阅读器另存为 PDF 或先转成 EPUB/TXT 后再导入"
                .to_string(),
        ),
    }
}

/// lopdf 对逐字绘制的页面每个字库单独成行；把「单行只两三个字」占主导的页面
/// 合并回连贯文本（行内换行视为排版而非语义，不受影响行多的页面按原样保留）。
fn merge_glyph_runs(page: &str) -> String {
    let lines: Vec<&str> = page.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 8 {
        return page.to_string();
    }
    let short = lines
        .iter()
        .filter(|l| l.trim().chars().count() <= 3)
        .count();
    if short * 5 >= lines.len() * 3 {
        lines.iter().map(|l| l.trim()).collect()
    } else {
        page.to_string()
    }
}

/// 去掉每页首尾重复出现的页眉页脚（书名、章节名、页码）。
fn strip_running_headers(pages: &[String]) -> Vec<String> {
    if pages.len() < 3 {
        return pages.to_vec();
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for page in pages {
        let lines = pdf_lines(page);
        let mut seen: Vec<String> = Vec::new();
        for line in lines
            .iter()
            .take(EDGE_LINES)
            .chain(lines.iter().rev().take(EDGE_LINES))
        {
            let key = normalize_heading(line);
            // 纯数字/过短的行不参与统计，避免把正文短句误判成页眉
            if key.chars().count() >= 2 && !seen.contains(&key) {
                seen.push(key.clone());
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    let threshold = (pages.len() as f64 * RUNNING_HEADER_RATIO).ceil() as usize;
    let repeated: Vec<String> = counts
        .into_iter()
        .filter(|(_, n)| *n >= threshold)
        .map(|(k, _)| k)
        .collect();
    pages
        .iter()
        .map(|page| {
            let lines: Vec<&str> = page.lines().collect();
            // 非空行在原行号序列中的位置（边缘判定基于非空行序号）
            let significant: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| !l.trim().is_empty())
                .map(|(i, _)| i)
                .collect();
            let total = significant.len();
            lines
                .iter()
                .enumerate()
                .filter(|(i, line)| {
                    let trimmed = line.trim();
                    // 空行原样保留：pdf-extract 输出的段落分隔依赖空行结构，
                    // 重组时塌缩成 \n 会导致后续 join_wrapped_lines 把整页并成一段
                    if trimmed.is_empty() {
                        return true;
                    }
                    let idx = significant.iter().position(|p| p == i).unwrap_or(0);
                    let at_edge = idx < EDGE_LINES || idx + EDGE_LINES >= total;
                    // 页码行（纯数字、- 42 -、第 3 页 等）位置固定且每页不同，直接剔除
                    if at_edge && is_page_number(trimmed) {
                        return false;
                    }
                    !(at_edge && repeated.contains(&normalize_heading(trimmed)))
                })
                .map(|(_, line)| *line)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

fn pdf_lines(page: &str) -> Vec<String> {
    page.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// 判断一行是否为孤立页码：`42`、`- 42 -`、`[42]`、`第 3 页`、`42/128` 等。
fn is_page_number(line: &str) -> bool {
    let t = line.trim();
    let core = t
        .trim_matches(|c: char| c == '-' || c == '—' || c == '–' || c.is_whitespace())
        .trim_matches(|c: char| ['[', ']', '(', ')'].contains(&c));
    if !core.is_empty() && core.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if let Some((num, total)) = core.split_once('/') {
        return !num.is_empty()
            && !total.is_empty()
            && num.chars().all(|c| c.is_ascii_digit())
            && total.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(mid) = t.strip_prefix('第').and_then(|r| r.strip_suffix('页')) {
        let mid = mid.trim();
        return !mid.is_empty()
            && mid
                .chars()
                .all(|c| c.is_ascii_digit() || "一二三四五六七八九十百零〇两".contains(c));
    }
    false
}

/// 标题比较用的归一化键：去空白、小写。
fn normalize_heading(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// 一页正文切段：接回排版换行、拼接连字符断词、归一化康熙部首，空行作为段落分隔。
fn pdf_page_paragraphs(page: &str) -> Vec<String> {
    let joined = join_wrapped_lines(page);
    let text = clean_pdf_text(&joined);
    text.split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// 把排版换行接回完整段落：段落内换行直接删掉（中英文都适用，PDF 的换行不代表语义分段），
/// 空行保留为段落分隔。
fn join_wrapped_lines(page: &str) -> String {
    let mut out = String::new();
    let mut prev_end: Option<char> = None;
    for raw in page.lines() {
        let line = raw.trim();
        if line.is_empty() {
            if !out.ends_with("\n\n") && !out.is_empty() {
                out.push_str("\n\n");
            }
            prev_end = None;
            continue;
        }
        if let Some(last) = prev_end {
            // 行尾是连字符：英文断词，直接拼接；否则按中英文分别处理
            if last == '-' && line.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                out.pop();
            } else if !needs_space(last, line.chars().next().unwrap_or(' ')) {
                // 中文之间不补空格
            } else {
                out.push(' ');
            }
        }
        out.push_str(line);
        prev_end = line.chars().last();
    }
    out
}

fn needs_space(prev: char, next: char) -> bool {
    prev.is_ascii_alphanumeric() && next.is_ascii_alphanumeric()
}

/// 归一化：康熙部首（U+2F00 起）→ 对应统一汉字，全角空格 → 半角，合并连续空白，
/// 并抹掉文本内混排的控制字符（部分 PDF 会在每个对象间插入 \u{0}..\u{1f}）。
/// 部分中文排版软件导出的 PDF 会把正文里的汉字映射到康熙部首区，不归一化会断句断词。
fn clean_pdf_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0usize;
    for ch in text.chars() {
        let mapped = match ch {
            '\u{2f00}'..='\u{2fd5}' => kangxi_to_unified(ch).unwrap_or(ch),
            '\u{3000}' => ' ',
            '\0'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}' => continue,
            c if c.is_whitespace() && c != '\n' => ' ',
            c => c,
        };
        if mapped == '\n' {
            blank_run += 1;
            if blank_run <= 2 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push(mapped);
        }
    }
    out
}

/// 康熙部首 U+2F00..U+2FD5 按码位顺序对应的统一汉字（214 个，与 NFKC 映射一致）。
const KANGXI_RADICALS: &str = "一丨丶丿乙亅二亠人儿入八冂冖冫几凵刀力勹匕匚匸十卜卩厂厶又口囗土士夂夊夕大女子宀寸小尢尸屮山巛工己巾干幺广廴廾弋弓彐彡彳心戈戶手支攴文斗斤方无日曰月木欠止歹殳毋比毛氏气水火爪父爻爿片牙牛犬玄玉瓜瓦甘生用田疋疒癶白皮皿目矛矢石示禸禾穴立竹米糸缶网羊羽老而耒耳聿肉臣自至臼舌舛舟艮色艸虍虫血行衣襾見角言谷豆豕豸貝赤走足身車辛辰辵邑酉釆里金長門阜隶隹雨靑非面革韋韭音頁風飛食首香馬骨高髟鬥鬯鬲鬼魚鳥鹵鹿麥麻黃黍黑黹黽鼎鼓鼠鼻齊齒龍龜龠";

fn kangxi_to_unified(ch: char) -> Option<char> {
    let offset = (ch as u32 - 0x2F00) as usize;
    KANGXI_RADICALS.chars().nth(offset)
}

/// 按标题切章：识别「第N章/节/部分」「Chapter N」「一、二、」式编号行；
/// 找不到标题时整本作为一章（由 group_units_into_chapters 兜底拆段）。
fn split_pdf_chapters(blocks: Vec<String>) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut title = "正文".to_string();
    let mut paragraphs: Vec<String> = Vec::new();
    for block in blocks {
        if is_pdf_heading(&block) {
            if !paragraphs.is_empty() {
                chapters.push(Chapter {
                    title: title.clone(),
                    paragraphs: std::mem::take(&mut paragraphs),
                });
            }
            title = block;
        } else {
            paragraphs.push(block);
        }
    }
    if !paragraphs.is_empty() {
        chapters.push(Chapter {
            title,
            paragraphs,
        });
    }
    chapters
}

fn is_pdf_heading(block: &str) -> bool {
    let t = block.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.is_empty() || chars.len() > MAX_HEADING_CHARS {
        return false;
    }
    // 标题通常不带句末标点
    if t.ends_with('。') || t.ends_with('，') || t.ends_with('；') || t.ends_with('.') {
        return false;
    }
    let has_newline = t.contains('\n');
    if has_newline {
        return false;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("chapter ") || lower.starts_with("part ") {
        return true;
    }
    if t.starts_with('第') {
        let rest = &t['第'.len_utf8()..];
        let end = rest.find(|c: char| !c.is_ascii_digit() && !"一二三四五六七八九十百零〇两".contains(c));
        if let Some(end) = end {
            let numeral = &rest[..end];
            let tail = &rest[end..];
            if !numeral.is_empty()
                && ["章", "节", "部分", "篇", "讲"]
                    .iter()
                    .any(|unit| tail.starts_with(unit))
            {
                return true;
            }
        }
    }
    // 「一、xxx」式编号标题
    if let Some(first) = chars.first() {
        if "一二三四五六七八九十".contains(*first)
            && chars.get(1) == Some(&'、')
            && chars.len() >= 3
        {
            return true;
        }
    }
    false
}

// ---------- EPUB ----------

#[cfg(test)]
pub fn parse_epub(bytes: &[u8]) -> Result<Vec<Chapter>, String> {
    Ok(parse_epub_grouped(bytes)?.chapters)
}

pub fn parse_epub_grouped(bytes: &[u8]) -> Result<ParsedBook, String> {
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("无法读取 EPUB 文件：{e}"))?;
    let container = read_zip_text(&mut zip, "META-INF/container.xml")
        .ok_or("EPUB 缺少 META-INF/container.xml")?;
    let opf_path = parse_container(&container).ok_or("EPUB 缺少 OPF 路径信息")?;
    let opf_dir = parent_dir(&opf_path);
    let opf = read_zip_text(&mut zip, &opf_path).ok_or_else(|| format!("EPUB 缺少 {opf_path}"))?;
    let opf_data = parse_opf(&opf);

    let mut toc: HashMap<String, String> = HashMap::new();
    // 目录层级：spine 文件 → 顶层目录条目标题（"第N章"即大节）
    let mut hier: HashMap<String, String> = HashMap::new();
    if let Some(toc_id) = &opf_data.toc_id {
        if let Some(href) = opf_data.manifest.get(toc_id) {
            let path = resolve_href(&opf_dir, href);
            if let Some(ncx) = read_zip_text(&mut zip, &path) {
                parse_ncx(&ncx, &opf_dir, &mut toc, &mut hier);
            }
        }
    }
    if let Some(href) = &opf_data.nav_href {
        let path = resolve_href(&opf_dir, href);
        if let Some(nav) = read_zip_text(&mut zip, &path) {
            parse_nav(&nav, &opf_dir, &mut toc, &mut hier);
        }
    }

    let mut chapters = Vec::new();
    let mut group_titles = Vec::new();
    // 图片登记：zip 内路径 → 输出文件名（去重，同一图片多处引用只存一份）
    let mut img_seq = 0usize;
    let mut img_names: HashMap<String, String> = HashMap::new();
    let mut img_pending: Vec<(String, String)> = Vec::new(); // (输出文件名, zip 内路径)
    for (i, idref) in opf_data.spine.iter().enumerate() {
        let Some(href) = opf_data.manifest.get(idref) else {
            continue;
        };
        let path = resolve_href(&opf_dir, href);
        let Some(xhtml) = read_zip_text(&mut zip, &path) else {
            continue;
        };
        let doc_dir = parent_dir(&path);
        let (paragraphs, doc_title) = {
            let mut register = |src: &str, _alt: &str| -> Option<String> {
                let src = src.trim();
                if src.is_empty() || src.starts_with("data:") {
                    return None;
                }
                let zip_path = resolve_href(&doc_dir, src);
                if let Some(name) = img_names.get(&zip_path) {
                    return Some(name.clone());
                }
                img_seq += 1;
                let base = zip_path.rsplit('/').next().unwrap_or("image");
                let name = format!("img_{}_{}", img_seq, sanitize_file_name(base));
                img_names.insert(zip_path.clone(), name.clone());
                img_pending.push((name.clone(), zip_path));
                Some(name)
            };
            xhtml_to_paragraphs(&xhtml, &mut register)
        };
        if paragraphs.is_empty() {
            continue;
        }
        let title = toc
            .get(&path)
            .cloned()
            .or_else(|| lookup_by_basename(&toc, &path))
            .or(doc_title)
            .unwrap_or_else(|| format!("第 {} 部分", i + 1));
        let group_title = hier.get(&path).cloned().unwrap_or_else(|| title.clone());
        chapters.push(Chapter { title, paragraphs });
        group_titles.push(group_title);
    }
    if chapters.is_empty() {
        return Err("EPUB 中没有可读的正文内容".to_string());
    }
    let mut images = Vec::new();
    for (name, zip_path) in img_pending {
        if let Some(bytes) = read_zip_bytes(&mut zip, &zip_path) {
            images.push((name, bytes));
        }
    }
    Ok(ParsedBook {
        chapters,
        group_titles,
        images,
    })
}

/// 图片落盘的安全文件名：只留字母数字和 . _ -，最长 60 字符。
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    if cleaned.is_empty() {
        "image".to_string()
    } else {
        cleaned
    }
}

fn read_zip_bytes<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    for candidate in zip_name_candidates(name) {
        if let Ok(mut file) = zip.by_name(&candidate) {
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                return Some(buf);
            }
        }
    }
    None
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
    /// EPUB2：`<meta name="cover" content="manifest-item-id"/>`
    cover_meta_id: Option<String>,
    /// EPUB3：manifest item properties 含 cover-image
    cover_href: Option<String>,
}

fn parse_opf(xml: &str) -> OpfData {
    let mut data = OpfData {
        manifest: HashMap::new(),
        spine: Vec::new(),
        toc_id: None,
        nav_href: None,
        cover_meta_id: None,
        cover_href: None,
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut section = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"metadata" => section = "metadata".to_string(),
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
                b"meta" if section == "metadata" => collect_cover_meta(&e, &mut data),
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
                b"metadata" | b"manifest" | b"spine" => section.clear(),
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

fn collect_cover_meta(e: &quick_xml::events::BytesStart<'_>, data: &mut OpfData) {
    let mut name = String::new();
    let mut content = String::new();
    for attr in e.attributes().flatten() {
        match local_name(attr.key.as_ref()) {
            b"name" => name = attr.unescape_value().map(|v| v.into_owned()).unwrap_or_default(),
            b"content" => {
                content = attr
                    .unescape_value()
                    .map(|v| v.into_owned())
                    .unwrap_or_default()
            }
            _ => {}
        }
    }
    if name.eq_ignore_ascii_case("cover") && !content.is_empty() {
        data.cover_meta_id = Some(content);
    }
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
        if properties
            .split_whitespace()
            .any(|p| p.eq_ignore_ascii_case("cover-image"))
        {
            data.cover_href = Some(href.clone());
        }
        data.manifest.insert(id, href);
    }
}

/// 提取 EPUB 内嵌封面：EPUB3 properties="cover-image" 优先，EPUB2 meta cover 其次。
/// 返回 (落盘文件名 cover.<ext>, 图片字节)；无封面返回 None。
pub fn extract_epub_cover(bytes: &[u8]) -> Option<(String, Vec<u8>)> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).ok()?;
    let container = read_zip_text(&mut zip, "META-INF/container.xml")?;
    let opf_path = parse_container(&container)?;
    let opf_dir = parent_dir(&opf_path);
    let opf = read_zip_text(&mut zip, &opf_path)?;
    let opf_data = parse_opf(&opf);
    let href = opf_data.cover_href.or_else(|| {
        opf_data
            .cover_meta_id
            .as_ref()
            .and_then(|id| opf_data.manifest.get(id).cloned())
    })?;
    let zip_path = resolve_href(&opf_dir, &href);
    let data = read_zip_bytes(&mut zip, &zip_path)?;
    let ext = match href.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "jpg" | "jpeg" => "jpg",
        "png" => "png",
        "webp" => "webp",
        "gif" => "gif",
        // 扩展名不可信时按魔数嗅探
        _ if data.starts_with(b"\xff\xd8") => "jpg",
        _ if data.starts_with(b"\x89PNG") => "png",
        _ if data.starts_with(b"RIFF") => "webp",
        _ if data.starts_with(b"GIF8") => "gif",
        _ => "jpg",
    };
    Some((format!("cover.{ext}"), data))
}

/// 封面约定：books/<id>/cover.<ext> 直接落书目录，不入库（不加列），
/// 前端按 BookSummary.cover_path 渲染，iOS 容器 UUID 变更不受影响。
fn write_cover_if_any(book_dir: &Path, source_file: &Path, format: &str) -> Option<String> {
    if format != "epub" {
        return None;
    }
    let bytes = std::fs::read(source_file).ok()?;
    let (name, data) = extract_epub_cover(&bytes)?;
    let dest = book_dir.join(&name);
    std::fs::write(&dest, data).ok()?;
    Some(dest.to_string_lossy().into_owned())
}

/// 列出书籍时的封面补建：已有 cover.* 直接回；EPUB 且源文件在则当场提取。
pub fn ensure_book_cover(
    app_data_dir: &Path,
    book_id: i64,
    source_path: &Path,
    format: &str,
) -> Option<String> {
    let book_dir = app_data_dir.join("books").join(book_id.to_string());
    if let Ok(entries) = std::fs::read_dir(&book_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("cover.") {
                return Some(e.path().to_string_lossy().into_owned());
            }
        }
    }
    write_cover_if_any(&book_dir, source_path, format)
}

/// 记录一个目录条目：扁平标题表 + 大节（顶层条目）映射；同一路径只取首次出现。
/// `stack` 为当前打开的标签链（栈底是顶层条目）。
fn record_nav_entry(
    stack: &[String],
    opf_dir: &str,
    src: &str,
    toc: &mut HashMap<String, String>,
    hier: &mut HashMap<String, String>,
) {
    let Some(label) = stack.last().filter(|s| !s.is_empty()).cloned() else {
        return;
    };
    let path = resolve_href(opf_dir, src);
    toc.entry(path.clone()).or_insert(label.clone());
    let root = if stack.len() > 1 {
        stack
            .first()
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or(label)
    } else {
        label
    };
    hier.entry(path).or_insert(root);
}

fn parse_ncx(
    xml: &str,
    opf_dir: &str,
    toc: &mut HashMap<String, String>,
    hier: &mut HashMap<String, String>,
) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    // 打开的 navPoint 标签栈（嵌套即目录层级）
    let mut stack: Vec<String> = Vec::new();
    let mut in_label_text = false;
    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"navPoint" => stack.push(String::new()),
                b"text" => in_label_text = true,
                b"content" => {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"src" {
                            if let Ok(v) = attr.unescape_value() {
                                record_nav_entry(&stack, opf_dir, &v, toc, hier);
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                // <content src="..."/> 常见于自闭合形式
                if local_name(e.name().as_ref()) == b"content" {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"src" {
                            if let Ok(v) = attr.unescape_value() {
                                record_nav_entry(&stack, opf_dir, &v, toc, hier);
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if in_label_text {
                    if let Some(top) = stack.last_mut() {
                        if top.is_empty() {
                            *top = t.unescape().unwrap_or_default().trim().to_string();
                        }
                    }
                    in_label_text = false;
                }
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"navPoint" => {
                    stack.pop();
                }
                b"text" => in_label_text = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }
}

fn parse_nav(
    xml: &str,
    opf_dir: &str,
    toc: &mut HashMap<String, String>,
    hier: &mut HashMap<String, String>,
) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    // <li> 标签栈（目录 nav 中嵌套 <ol> 即层级）
    let mut li_stack: Vec<String> = Vec::new();
    let mut pending_href: Option<String> = None;
    // 只把 epub:type="toc" 的 nav 计入层级；扁平表保持原样全收
    let mut in_toc_nav = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"nav" => {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"type" {
                            if let Ok(v) = attr.unescape_value() {
                                in_toc_nav = v.trim().eq_ignore_ascii_case("toc");
                            }
                        }
                    }
                }
                b"li" => li_stack.push(String::new()),
                b"a" => {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"href" {
                            pending_href = attr.unescape_value().ok().map(|v| v.into_owned());
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                if let Some(href) = pending_href.take() {
                    let title = t.unescape().unwrap_or_default().trim().to_string();
                    if !title.is_empty() {
                        let path = resolve_href(opf_dir, &href);
                        toc.entry(path.clone()).or_insert(title.clone());
                        if in_toc_nav {
                            if let Some(top) = li_stack.last_mut() {
                                if top.is_empty() {
                                    *top = title.clone();
                                }
                            }
                            let root = if li_stack.len() > 1 {
                                li_stack
                                    .first()
                                    .filter(|s| !s.is_empty())
                                    .cloned()
                                    .unwrap_or(title)
                            } else {
                                title
                            };
                            hier.entry(path).or_insert(root);
                        }
                    }
                }
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"li" => {
                    li_stack.pop();
                }
                b"a" => pending_href = None,
                b"nav" => in_toc_nav = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        buf.clear();
    }
}

/// 把 XHTML 去标签成段落，同时顺带取 <title>；遇到 <img> 经 register_img 登记
/// （返回输出文件名）后在段间插入 `![alt](文件名)` 标记段，供前端原位还原插图。
fn xhtml_to_paragraphs(
    html: &str,
    register_img: &mut dyn FnMut(&str, &str) -> Option<String>,
) -> (Vec<String>, Option<String>) {
    let mut reader = Reader::from_str(html);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut title = String::new();
    let mut capture_title = false;
    let mut skip_depth = 0i32;
    let mut in_head = false;

    fn collect_img(
        e: &quick_xml::events::BytesStart<'_>,
        paragraphs: &mut Vec<String>,
        current: &mut String,
        register_img: &mut dyn FnMut(&str, &str) -> Option<String>,
    ) {
        let (mut src, mut alt) = (String::new(), String::new());
        for attr in e.attributes().flatten() {
            match local_name(attr.key.as_ref()) {
                b"src" => {
                    src = attr
                        .unescape_value()
                        .map(|v| v.into_owned())
                        .unwrap_or_default()
                }
                b"alt" => {
                    alt = attr
                        .unescape_value()
                        .map(|v| v.into_owned())
                        .unwrap_or_default()
                }
                _ => {}
            }
        }
        if let Some(name) = register_img(&src, &alt) {
            flush_paragraph(paragraphs, current);
            let alt = alt.replace(['[', ']'], "").replace('\n', " ");
            paragraphs.push(format!("![{}]({name})", alt.trim()));
        }
    }

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"style" | b"script" => skip_depth += 1,
                b"head" => in_head = true,
                b"title" => capture_title = true,
                b"body" => in_head = false,
                b"img" => collect_img(&e, &mut paragraphs, &mut current, register_img),
                name if is_block_tag(name) => flush_paragraph(&mut paragraphs, &mut current),
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if name == b"img" {
                    collect_img(&e, &mut paragraphs, &mut current, register_img);
                } else if is_block_tag(name) {
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

/// 把"每 spine 章的大节标题"展开成"每个学习单元的大节标题"：
/// 用与导入完全相同的切分复算对齐单元顺序；数量对不上（源文件已变化）返回 None。
pub fn unit_group_titles(parsed: &ParsedBook, unit_count: usize) -> Option<Vec<String>> {
    if parsed.chapters.len() != parsed.group_titles.len() {
        return None;
    }
    let mut out = Vec::new();
    for (chapter, group_title) in parsed.chapters.iter().zip(parsed.group_titles.iter()) {
        for _ in chunk_paragraphs(&chapter.paragraphs) {
            out.push(group_title.clone());
        }
    }
    if out.len() == unit_count {
        Some(out)
    } else {
        None
    }
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

// ---------- 章节分组（阅读/领读以"章"为单位；元数据不入库，按标题推导） ----------

/// 一章总字符数上限：超过则拆成连续分段，避免阅读页和 LLM 输入过大。
pub const MAX_CHAPTER_CHARS: usize = 30000;

/// 剥掉单元标题尾部的全角编号后缀（"章名（2）" → "章名"），无后缀则原样返回。
pub fn chapter_group_title(unit_title: &str) -> &str {
    let trimmed = unit_title.trim_end();
    let Some(rest) = trimmed.strip_suffix('）') else {
        return unit_title;
    };
    let Some(pos) = rest.rfind('（') else {
        return unit_title;
    };
    let digits = &rest[pos + '（'.len_utf8()..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return unit_title;
    }
    let base = unit_title[..pos].trim_end();
    if base.is_empty() {
        unit_title
    } else {
        base
    }
}

/// 一段连续阅读章节：聚合同名小节得到；`unit_ids` 按书内顺序排列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterRun {
    pub index: usize,
    pub title: String,
    pub unit_ids: Vec<i64>,
}

/// 按 idx 顺序把连续同大节标题的单元聚成一章；累计字符数达到 MAX_CHAPTER_CHARS
/// 时强制另起一段（第 2 段标题追加"（续）"、第 3 段"（续 2）"……），
/// txt/pdf 整书一章的场景由此兜底。输入为 (unit_id, idx, 大节标题, 内容字符数)，
/// 大节标题由调用方给出（目录层级或 chapter_group_title 推导）。
pub fn group_units_into_chapters(units: &[(i64, i64, String, usize)]) -> Vec<ChapterRun> {
    let mut groups: Vec<ChapterRun> = Vec::new();
    let mut last_base = String::new();
    let mut current_len = 0usize;
    // 当前 base 已拆出的段数（仅因超长拆分时递增）
    let mut split_count = 0usize;
    for (id, _idx, title, char_count) in units {
        let base = title.as_str();
        if !groups.is_empty() && last_base == base && current_len < MAX_CHAPTER_CHARS {
            groups.last_mut().unwrap().unit_ids.push(*id);
            current_len += char_count;
            continue;
        }
        let run_title = if !groups.is_empty() && last_base == base {
            split_count += 1;
            if split_count == 1 {
                format!("{base}（续）")
            } else {
                format!("{base}（续 {split_count}）")
            }
        } else {
            split_count = 0;
            base.to_string()
        };
        last_base = base.to_string();
        current_len = *char_count;
        groups.push(ChapterRun {
            index: groups.len(),
            title: run_title,
            unit_ids: vec![*id],
        });
    }
    groups
}

/// 喂给 LLM 的正文上限：超长时保留头 60% + 尾 40%，中间标注省略。
pub fn cap_llm_input(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let head = max_chars * 3 / 5;
    let tail = max_chars - head;
    let head_s: String = text.chars().take(head).collect();
    let tail_s: String = text.chars().skip(total - tail).collect();
    format!("{head_s}\n……（中间内容略）……\n{tail_s}")
}

// ---------- 导入 ----------

/// iOS/macOS 系统文件选择器可能回传 `file:///private/…` 形式的 URL（带协议头、
/// CJK 等非 ASCII 按 %XX 编码），直接当文件系统路径会判为「文件不存在」。
/// 统一规范化：剥 `file:` 协议头并做百分号解码。普通路径原样返回。
fn normalize_import_path(path: &str) -> String {
    let stripped = path
        .strip_prefix("file://")
        .or_else(|| path.strip_prefix("file:"))
        .unwrap_or(path)
        .to_string();
    percent_decode(&stripped)
}

pub fn import_book(db: &Db, app_data_dir: &Path, path: &str) -> Result<BookSummary, String> {
    let path = normalize_import_path(path);
    let file_path = Path::new(&path);
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
        .find_book_by_source_path(&path)
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
    let parsed = parse_file_grouped(file_path)?;
    let units = chunk_chapters(parsed.chapters);
    if units.is_empty() {
        return Err("未能从该文件中解析出任何学习内容".to_string());
    }

    // 书名+单元数双判定防重复导入：源文件落库后会被拷贝进 books/<id>/，
    // 单凭 source_path 去重拦不住「同一本书换了路径再来一次」
    if db
        .list_books()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|b| b.title == title && b.total_units == units.len() as i64)
    {
        return Err("这本书已经导入过了".to_string());
    }

    let added_at = chrono::Local::now().format("%Y-%m-%d").to_string();
    let book_id = db
        .insert_book(&title, &path, format, &added_at)
        .map_err(|e| e.to_string())?;
    let book_dir = app_data_dir.join("books").join(book_id.to_string());
    if let Err(e) = persist_units(db, book_id, &book_dir, &units, &parsed.images) {
        let _ = db.delete_book(book_id);
        let _ = std::fs::remove_dir_all(&book_dir);
        return Err(e);
    }

    // 源文件自包含：拷贝进 books/<id>/ 并把 source_path 指向副本。
    // iOS 数据容器每次安装换 UUID（绝对路径随之失效）、Inbox/tmp 也会被系统清理，
    // 不拷贝则重装后读时重解析源文件拿不到（章划分降级标题推导）
    if let Some(name) = file_path.file_name() {
        let copy_path = book_dir.join(name);
        if std::fs::copy(file_path, &copy_path).is_ok() {
            let _ = db.update_book_source_path(book_id, &copy_path.to_string_lossy());
        }
    }

    // EPUB 内嵌封面提取到 books/<id>/cover.<ext>（无封面的格式静默跳过）
    let cover_path = write_cover_if_any(&book_dir, file_path, format);

    Ok(BookSummary {
        id: book_id,
        title,
        format: format.to_string(),
        added_at,
        total_units: units.len() as i64,
        done_units: 0,
        cover_path,
    })
}

fn persist_units(
    db: &Db,
    book_id: i64,
    book_dir: &Path,
    units: &[UnitDraft],
    images: &[(String, Vec<u8>)],
) -> Result<(), String> {
    std::fs::create_dir_all(book_dir).map_err(|e| format!("无法创建书籍目录：{e}"))?;
    if !images.is_empty() {
        let images_dir = book_dir.join("images");
        std::fs::create_dir_all(&images_dir).map_err(|e| format!("无法创建图片目录：{e}"))?;
        for (name, bytes) in images {
            std::fs::write(images_dir.join(name), bytes)
                .map_err(|e| format!("无法写入图片 {name}：{e}"))?;
        }
    }
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

    #[test]
    fn normalize_import_path_handles_file_urls() {
        // 普通路径原样返回
        assert_eq!(normalize_import_path("/tmp/书.md"), "/tmp/书.md");
        // file:// 协议头剥除
        assert_eq!(normalize_import_path("file:///tmp/a.txt"), "/tmp/a.txt");
        assert_eq!(normalize_import_path("file:/tmp/a.txt"), "/tmp/a.txt");
        // iOS 选择器回传：协议头 + CJK 百分号编码
        assert_eq!(
            normalize_import_path(
                "file:///private/var/mobile/Containers/Shared/AppGroup/Inbox/%E5%A4%A7%E6%A8%A1%E5%9E%8B.pdf"
            ),
            "/private/var/mobile/Containers/Shared/AppGroup/Inbox/大模型.pdf"
        );
        // 非法百分号序列原样保留
        assert_eq!(normalize_import_path("file:///tmp/a%zz.md"), "/tmp/a%zz.md");
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
    fn pdf_kangxi_radicals_normalized_to_unified() {
        // 「用用一文」：这些字在部分中文 PDF 里被映射到康熙部首区
        let raw = "\u{2F64}\u{2F64}\u{2F00}\u{2F42}";
        assert_eq!(clean_pdf_text(raw), "用用一文");
        // 214 个部首全部有映射，且首尾对齐
        assert_eq!(kangxi_to_unified('\u{2F00}'), Some('一'));
        assert_eq!(kangxi_to_unified('\u{2F64}'), Some('用'));
        assert_eq!(kangxi_to_unified('\u{2FD5}'), Some('龠'));
        assert_eq!(KANGXI_RADICALS.chars().count(), 214);
    }

    #[test]
    fn pdf_soft_line_breaks_join_into_one_paragraph() {
        let page = "这是一段被排版切断的中文\n接着下一行继续。\n\n新的段落在这里。";
        let blocks = pdf_page_paragraphs(page);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], "这是一段被排版切断的中文接着下一行继续。");
        assert_eq!(blocks[1], "新的段落在这里。");
    }

    #[test]
    fn pdf_hyphenated_wrap_joined() {
        let page = "The extraor-\ndinary result";
        assert_eq!(pdf_page_paragraphs(page), vec!["The extraordinary result"]);
    }

    #[test]
    fn pdf_running_headers_removed() {
        let page = |body: &str| format!("深入理解计算机系统\n{body}\n- 42 -");
        let pages = vec![
            page("正文一。"),
            page("正文二。"),
            page("正文三。"),
            page("正文四。"),
        ];
        let stripped = strip_running_headers(&pages);
        assert!(stripped.iter().all(|p| !p.contains("深入理解计算机系统")));
        assert!(stripped.iter().all(|p| !p.contains("- 42 -")));
        assert!(stripped[0].contains("正文一。"));
    }

    #[test]
    fn pdf_short_documents_keep_their_lines() {
        // 少于 3 页不做页眉剔除，避免误删
        let pages = vec!["书名\n正文".to_string(), "书名\n正文二".to_string()];
        let stripped = strip_running_headers(&pages);
        assert_eq!(stripped, pages);
    }

    #[test]
    fn pdf_chapters_split_on_chinese_headings() {
        let blocks: Vec<String> = vec![
            "第一章 绪论".to_string(),
            "本章介绍基本概念。".to_string(),
            "第二章 方法".to_string(),
            "方法部分正文。".to_string(),
            "一、实验设置".to_string(),
            "实验内容。".to_string(),
        ];
        let chapters = split_pdf_chapters(blocks);
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "第一章 绪论");
        assert_eq!(chapters[1].title, "第二章 方法");
        assert_eq!(chapters[2].title, "一、实验设置");
        assert_eq!(chapters[2].paragraphs, vec!["实验内容。"]);
    }

    #[test]
    fn pdf_chapters_split_on_english_headings() {
        let blocks: Vec<String> = vec![
            "Chapter 1 Introduction".to_string(),
            "Intro body.".to_string(),
            "Chapter 2 Method".to_string(),
            "Method body.".to_string(),
        ];
        let chapters = split_pdf_chapters(blocks);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "Chapter 1 Introduction");
        assert_eq!(chapters[1].title, "Chapter 2 Method");
    }

    #[test]
    fn pdf_without_headings_stays_single_chapter() {
        let blocks: Vec<String> = vec![
            "这是一段普通正文，没有任何标题结构。".to_string(),
            "第二段内容继续。".to_string(),
        ];
        let chapters = split_pdf_chapters(blocks);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "正文");
        assert_eq!(chapters[0].paragraphs.len(), 2);
    }

    #[test]
    fn pdf_sentence_like_lines_are_not_headings() {
        // 以「第」开头但不是章节编号（如「第一，我们需要……」）不应误判为标题
        assert!(!is_pdf_heading("第一，我们需要重新思考这个问题。"));
        assert!(!is_pdf_heading("第三方向。"));
        assert!(!is_pdf_heading(&"很长的正文行".repeat(10)));
        assert!(is_pdf_heading("第一章 绪论"));
        assert!(is_pdf_heading("第三部分 实践"));
        assert!(is_pdf_heading("Chapter 3"));
    }

    #[test]
    fn pdf_control_characters_stripped() {
        // 实测部分 PDF（Pro 6000D 测试报告）逐对象输出后残留 \u{1} 等控制字符
        assert_eq!(clean_pdf_text("Pro\u{1}6000D\u{1}异构\u{7f}测试"), "Pro6000D异构测试");
        assert_eq!(clean_pdf_text("正常文本\u{0}继续"), "正常文本继续");
    }

    #[test]
    fn pdf_page_number_lines_detected() {
        assert!(is_page_number("42"));
        assert!(is_page_number("- 42 -"));
        assert!(is_page_number("— 128 —"));
        assert!(is_page_number("[7]"));
        assert!(is_page_number("3/128"));
        assert!(is_page_number("第 3 页"));
        assert!(is_page_number("第 二十 页"));
        // 正文里的数字不应误判
        assert!(!is_page_number("第 3 页讲的是要点"));
        assert!(!is_page_number("42 条规则"));
        assert!(!is_page_number("第 42 题"));
    }

    #[test]
    fn pdf_strip_preserves_blank_line_structure() {
        // pdf-extract 输出的段落分隔依赖空行（\n\n）；strip 重组时若塌缩成 \n，
        // 会把整页并成一段、标题识别全灭（实测 Alluxio 51 章掉到 1 章）
        let pages = vec![
            "一、章节标题\n\n正文段一。\n\n正文段二。".to_string(),
            "一、另一标题\n\n正文段三。".to_string(),
            "一、第三标题\n\n正文段四。".to_string(),
        ];
        let stripped = strip_running_headers(&pages);
        assert_eq!(stripped, pages, "无页眉页码时输出必须与输入一致");
    }

    #[test]
    fn pdf_page_numbers_removed_at_edges() {
        let page = |n: i32, body: &str| format!("书名页眉\n{body}\n- {n} -");
        let pages = vec![
            page(1, "第一段。"),
            page(2, "第二段。"),
            page(3, "第三段。"),
        ];
        let stripped = strip_running_headers(&pages);
        assert!(stripped.iter().all(|p| !p.contains('1')));
        assert!(stripped.iter().any(|p| p.contains("第一段。")));
    }

    #[test]
    fn pdf_glyph_runs_merged() {
        // 逐字绘制的页面：lopdf 提取结果每行一个字，应合并回连贯文本
        let page = (1..=10).map(|_| "字 \n").collect::<String>();
        assert_eq!(merge_glyph_runs(&page), "字字字字字字字字字字");

        // 正常版式的行距大得多，不应合并
        let normal = "第一行有十个字符以上啊\n第二行内容也很长很长\n第三行同样是个完整句子\n第四行结束\n第五行开始\n第六行中间\n第七行后面\n第八行末尾\n第九行收尾\n第十行最后";
        assert_eq!(merge_glyph_runs(normal), normal);

        // 行数过少不启用（避免误判）
        let few = "字\n字\n字";
        assert_eq!(merge_glyph_runs(few), few);
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
    fn parse_epub_extracts_images_as_marker_paragraphs() {
        use std::io::Write;
        let png = b"\x89PNG-fake".to_vec();
        let page = |body: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>页</title></head><body>{body}</body></html>"#
            )
        };
        // 图片相对 spine 文档所在目录解析（OEBPS/text/ → ../figs/）
        let page1 = page("<p>图片前的一段。</p><img src=\"../figs/f1.png\" alt=\"图1 流程\"/><p>图后的一段。</p>");
        let page2 = page("<p>第二章正文。</p><img src=\"../figs/f1.png\"/>");
        let files: Vec<(&str, Vec<u8>)> = vec![
            ("META-INF/container.xml", CONTAINER_XML.as_bytes().to_vec()),
            ("OEBPS/content.opf", OPF_WITH_NCX.as_bytes().to_vec()),
            ("OEBPS/toc.ncx", NCX.as_bytes().to_vec()),
            ("OEBPS/text/a1.xhtml", page1.into_bytes()),
            ("OEBPS/text/a2.xhtml", page2.into_bytes()),
            ("OEBPS/figs/f1.png", png.clone()),
        ];
        let mut cur = Cursor::new(Vec::new());
        {
            let mut w = zip::write::ZipWriter::new(&mut cur);
            let opt = zip::write::SimpleFileOptions::default();
            for (n, c) in &files {
                w.start_file(*n, opt).unwrap();
                w.write_all(c).unwrap();
            }
            w.finish().unwrap();
        }
        let buf = cur.into_inner();

        let parsed = parse_epub_grouped(&buf).unwrap();
        // 同一图片两处引用只抽取一份，正文原位留下标记段
        assert_eq!(parsed.images.len(), 1);
        assert_eq!(parsed.images[0].1, png);
        let name = &parsed.images[0].0;
        assert!(name.ends_with("_f1.png"), "输出文件名应保留原文件名：{name}");
        let p1 = &parsed.chapters[0].paragraphs;
        assert!(
            p1.iter().any(|p| p == &format!("![图1 流程]({name})")),
            "{p1:?}"
        );
        assert!(!p1.iter().any(|p| p.contains("figs/")), "zip 内路径不应出现在正文：{p1:?}");
        assert!(parsed.chapters[1].paragraphs.iter().any(|p| p == &format!("![]({name})")));

        // 导入后图片写入 books/<id>/images/，单元文本保留标记
        let dir = temp_dir("img-import");
        let db = Db::open_in_memory().unwrap();
        let epub_path = dir.join("img-book.epub");
        std::fs::write(&epub_path, &buf).unwrap();
        let summary = import_book(&db, &dir, epub_path.to_string_lossy().as_ref()).unwrap();
        let images_dir = dir.join("books").join(summary.id.to_string()).join("images");
        assert!(images_dir.join(name).exists());
        let unit_text = std::fs::read_to_string(
            dir.join("books")
                .join(summary.id.to_string())
                .join("unit_0.txt"),
        )
        .unwrap();
        assert!(unit_text.contains(&format!("]({name})")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_epub_with_cover(opf: &str) -> Vec<u8> {
        let mut files: Vec<(&str, &[u8])> = vec![
            ("META-INF/container.xml", CONTAINER_XML.as_bytes()),
            ("OEBPS/content.opf", opf.as_bytes()),
        ];
        let page = xhtml("正文", &["第一章的正文第一段，讲述起点。"]);
        let page_bytes = page.as_bytes().to_vec();
        let png = b"\x89PNG-cover-fake".to_vec();
        files.push(("OEBPS/text/a1.xhtml", &page_bytes));
        files.push(("OEBPS/pics/cover.png", &png));
        let mut cur = Cursor::new(Vec::new());
        {
            let mut w = zip::write::ZipWriter::new(&mut cur);
            let opt = zip::write::SimpleFileOptions::default();
            for (n, c) in &files {
                w.start_file(*n, opt).unwrap();
                std::io::Write::write_all(&mut w, c).unwrap();
            }
            w.finish().unwrap();
        }
        cur.into_inner()
    }

    const OPF_COVER_EPUB2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>封面书</dc:title>
    <meta name="cover" content="cv"/>
  </metadata>
  <manifest>
    <item id="c1" href="text/a1.xhtml" media-type="application/xhtml+xml"/>
    <item id="cv" href="pics/cover.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;

    const OPF_COVER_EPUB3: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="id">
  <metadata><dc:title xmlns:dc="http://nope">封面书</dc:title></metadata>
  <manifest>
    <item id="c1" href="text/a1.xhtml" media-type="application/xhtml+xml"/>
    <item id="cv3" href="pics/cover.png" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;

    #[test]
    fn epub_cover_extracted_epub2_meta_and_epub3_properties() {
        for opf in [OPF_COVER_EPUB2, OPF_COVER_EPUB3] {
            let bytes = make_epub_with_cover(opf);
            let (name, data) = extract_epub_cover(&bytes).expect("应提取到封面");
            assert_eq!(name, "cover.png");
            assert_eq!(data, b"\x89PNG-cover-fake");
        }
        // 无封面声明的书静默返回 None
        let no_cover = OPF_COVER_EPUB3
            .replace(r#" properties="cover-image""#, "")
            .replace(r#"<item id="cv3" href="pics/cover.png" media-type="image/png"/>"#, "");
        assert!(extract_epub_cover(&make_epub_with_cover(&no_cover)).is_none());
    }

    #[test]
    fn import_epub_persists_cover_and_ensure_backfills() {
        let dir = temp_dir("cover-import");
        let db = Db::open_in_memory().unwrap();
        let epub_path = dir.join("with-cover.epub");
        std::fs::write(&epub_path, make_epub_with_cover(OPF_COVER_EPUB2)).unwrap();
        let summary = import_book(&db, &dir, epub_path.to_string_lossy().as_ref()).unwrap();
        let cover = summary.cover_path.expect("导入后应有封面路径");
        let cover_file = dir
            .join("books")
            .join(summary.id.to_string())
            .join("cover.png");
        assert!(cover_file.exists());
        assert_eq!(cover, cover_file.to_string_lossy());
        // 删掉后 ensure 按源文件（books/<id>/ 内自包含副本）补建
        std::fs::remove_file(&cover_file).unwrap();
        let src = dir
            .join("books")
            .join(summary.id.to_string())
            .join("with-cover.epub");
        let rebuilt = ensure_book_cover(&dir, summary.id, &src, "epub").expect("应补建封面");
        assert_eq!(rebuilt, cover_file.to_string_lossy());
        // 已有封面时直接命中 FASTPATH，不重提取
        let again = ensure_book_cover(&dir, summary.id, Path::new("/nonexistent"), "epub");
        assert_eq!(again, Some(cover_file.to_string_lossy().into_owned()));
        let _ = std::fs::remove_dir_all(&dir);
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

    const OPF_WITH_NCX_4: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
  <metadata><dc:title>层级书</dc:title></metadata>
  <manifest>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/s1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c3" href="text/s2.xhtml" media-type="application/xhtml+xml"/>
    <item id="c4" href="text/s3.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="c1"/>
    <itemref idref="c2"/>
    <itemref idref="c3"/>
    <itemref idref="c4"/>
  </spine>
</package>"#;

    const NCX_NESTED: &str = r#"<?xml version="1.0"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <docTitle><text>层级书</text></docTitle>
  <navMap>
    <navPoint id="n1"><navLabel><text>第1章 基础</text></navLabel><content src="text/ch1.xhtml"/>
      <navPoint id="n2"><navLabel><text>1.1 感知机</text></navLabel><content src="text/s1.xhtml"/></navPoint>
      <navPoint id="n3"><navLabel><text>1.2 回归</text></navLabel><content src="text/s2.xhtml"/></navPoint>
    </navPoint>
    <navPoint id="n4"><navLabel><text>第2章 硬件</text></navLabel><content src="text/s3.xhtml"/></navPoint>
  </navMap>
</ncx>"#;

    #[test]
    fn parse_epub_nested_ncx_maps_sections_to_chapter() {
        let bytes = write_zip(&[
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", OPF_WITH_NCX_4),
            ("OEBPS/toc.ncx", NCX_NESTED),
            ("OEBPS/text/ch1.xhtml", &xhtml("封面", &["第1章引言。"])),
            ("OEBPS/text/s1.xhtml", &xhtml("感知机", &["感知机内容。"])),
            ("OEBPS/text/s2.xhtml", &xhtml("回归", &["回归内容。"])),
            ("OEBPS/text/s3.xhtml", &xhtml("硬件", &["硬件内容。"])),
        ]);
        let parsed = parse_epub_grouped(&bytes).unwrap();
        // 扁平标题仍是各节目录名
        assert_eq!(parsed.chapters[1].title, "1.1 感知机");
        assert_eq!(parsed.chapters[2].title, "1.2 回归");
        // 大节归属：前三节同属"第1章 基础"，末节自成一章
        assert_eq!(
            parsed.group_titles,
            vec![
                "第1章 基础".to_string(),
                "第1章 基础".to_string(),
                "第1章 基础".to_string(),
                "第2章 硬件".to_string(),
            ]
        );
    }

    const OPF_WITH_NAV: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata><dc:title>层级书</dc:title></metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/s1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>"#;

    const NAV_NESTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<body>
<nav epub:type="toc"><ol>
  <li><a href="text/ch1.xhtml">第1章 基础</a><ol>
    <li><a href="text/s1.xhtml">1.1 感知机</a></li>
  </ol></li>
</ol></nav>
<nav epub:type="page-list"><ol>
  <li><a href="text/ch1.xhtml">1</a></li>
</ol></nav>
</body>
</html>"#;

    #[test]
    fn parse_epub_nested_nav_uses_only_toc_nav_for_hierarchy() {
        let bytes = write_zip(&[
            ("META-INF/container.xml", CONTAINER_XML),
            ("OEBPS/content.opf", OPF_WITH_NAV),
            ("OEBPS/nav.xhtml", NAV_NESTED),
            ("OEBPS/text/ch1.xhtml", &xhtml("封面", &["第1章引言。"])),
            ("OEBPS/text/s1.xhtml", &xhtml("感知机", &["感知机内容。"])),
        ]);
        let parsed = parse_epub_grouped(&bytes).unwrap();
        assert_eq!(
            parsed.group_titles,
            vec!["第1章 基础".to_string(), "第1章 基础".to_string()]
        );
    }

    #[test]
    fn unit_group_titles_align_with_chunking() {
        // 一章被切成两个单元时，两单元共享同一大节标题
        let long_para = "字".repeat(1600);
        let another = "段".repeat(1600);
        let parsed = ParsedBook {
            chapters: vec![
                Chapter {
                    title: "第1章".to_string(),
                    paragraphs: vec![long_para, another],
                },
                Chapter {
                    title: "第2章".to_string(),
                    paragraphs: vec!["短内容。".to_string()],
                },
            ],
            group_titles: vec!["第1章".to_string(), "第2章".to_string()],
            images: Vec::new(),
        };
        let titles = unit_group_titles(&parsed, 3).unwrap();
        assert_eq!(titles, vec!["第1章", "第1章", "第2章"]);
        // 单元数对不上（源文件变了）→ None，调用方回退
        assert!(unit_group_titles(&parsed, 2).is_none());
        assert!(unit_group_titles(&parsed, 4).is_none());
    }

    #[test]
    fn parse_file_grouped_markdown_is_flat() {
        let text = std::fs::read_to_string(fixture_path()).unwrap();
        let dir = temp_dir("mdgroup");
        let path = dir.join("book.md");
        std::fs::write(&path, text).unwrap();
        let parsed = parse_file_grouped(&path).unwrap();
        assert_eq!(parsed.chapters.len(), 3);
        assert_eq!(parsed.group_titles.len(), 3);
        for (ch, gt) in parsed.chapters.iter().zip(parsed.group_titles.iter()) {
            assert_eq!(&ch.title, gt);
        }
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn chapter_group_title_strips_numeric_suffix() {
        assert_eq!(chapter_group_title("长章（1）"), "长章");
        assert_eq!(chapter_group_title("长章（12）"), "长章");
        assert_eq!(chapter_group_title("长章"), "长章");
        // 非数字括号、半角括号、剥后会变空的都不处理
        assert_eq!(chapter_group_title("物种（修订版）"), "物种（修订版）");
        assert_eq!(chapter_group_title("章名(1)"), "章名(1)");
        assert_eq!(chapter_group_title("（1）"), "（1）");
        assert_eq!(chapter_group_title("长章（3） "), "长章");
    }

    #[test]
    fn group_units_consecutive_same_chapter() {
        let units = vec![
            (1, 0, "甲".to_string(), 2000),
            (2, 1, "甲".to_string(), 2000),
            (3, 2, "乙".to_string(), 1000),
            (4, 3, "丙（1）".to_string(), 500),
        ];
        let groups = group_units_into_chapters(&units);
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[0],
            ChapterRun {
                index: 0,
                title: "甲".to_string(),
                unit_ids: vec![1, 2]
            }
        );
        assert_eq!(groups[1].unit_ids, vec![3]);
        assert_eq!(groups[2].title, "丙（1）");
        assert_eq!(groups[2].unit_ids, vec![4]);

        // 空输入与单节书
        assert!(group_units_into_chapters(&[]).is_empty());
        let single = group_units_into_chapters(&[(7, 0, "正文".to_string(), 800)]);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].unit_ids, vec![7]);
    }

    #[test]
    fn group_units_splits_oversized_chapter() {
        let units: Vec<(i64, i64, String, usize)> = (0..10)
            .map(|i| (i as i64, i as i64, "正文".to_string(), 10000))
            .collect();
        let groups = group_units_into_chapters(&units);
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].title, "正文");
        assert_eq!(groups[0].unit_ids, vec![0, 1, 2]);
        assert_eq!(groups[1].title, "正文（续）");
        assert_eq!(groups[2].title, "正文（续 2）");
        assert_eq!(groups[3].title, "正文（续 3）");
        assert_eq!(groups[3].unit_ids, vec![9]);
        // 换章后拆段计数复位
        let mut tail = units.clone();
        tail.push((10, 10, "新章".to_string(), 100));
        let groups = group_units_into_chapters(&tail);
        assert_eq!(groups.last().unwrap().title, "新章");
    }

    #[test]
    fn cap_llm_input_truncates_middle() {
        let short = "短文不超上限";
        assert_eq!(cap_llm_input(short, 100), short);

        let head = "头".repeat(6000);
        let mid = "中".repeat(10000);
        let tail = "尾".repeat(4000);
        let text = format!("{head}{mid}{tail}");
        let capped = cap_llm_input(&text, 10000);
        assert!(capped.contains("（中间内容略）"));
        assert_eq!(capped.chars().filter(|c| *c == '头').count(), 6000);
        assert_eq!(capped.chars().filter(|c| *c == '尾').count(), 4000);
        // 头尾恰好衔接中间段之外的内容：不夹带被略去的中间正文
        let (front, back) = capped.split_once("（中间内容略）").unwrap();
        assert!(front.trim_end_matches(['…', '\n']).ends_with("头"));
        assert!(back.trim_start_matches(['…', '\n']).starts_with("尾"));
    }
}






