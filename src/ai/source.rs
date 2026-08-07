use std::{
    collections::HashSet,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};

const MAX_FILES: usize = 2_000;
const MAX_FILE_CHARS: usize = 4_000_000;
const MAX_TOTAL_CHARS: usize = 120_000_000;

const TEXT_EXTENSIONS: &[&str] = &[
    "md", "markdown", "mdown", "txt", "text", "html", "htm", "xhtml", "xml", "json", "jsonl",
    "csv", "tsv", "yaml", "yml", "toml", "rs", "py", "js", "jsx", "ts", "tsx", "java", "go", "c",
    "h", "cpp", "hpp", "cc", "cxx", "css", "scss", "sql", "sh", "bash", "zsh", "fish", "log",
    "tex", "rst", "epub",
];

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "svg", "ico",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSource {
    pub path: PathBuf,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct SourceBundle {
    pub root: PathBuf,
    pub text: String,
    pub images: Vec<ImageSource>,
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl SourceBundle {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.images.is_empty()
    }
}

/// Recursively scans a local file or directory into deterministic, bounded
/// input for the AI import workflow.
pub fn scan_path(path: impl AsRef<Path>) -> Result<SourceBundle> {
    let path = path.as_ref();
    let root = path
        .canonicalize()
        .map_err(|error| anyhow!("无法访问本地路径 {}: {error}", path.display()))?;
    let mut files = Vec::new();
    collect_files(&root, &mut files)?;
    files.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
    files.dedup();

    if files.len() > MAX_FILES {
        return Err(anyhow!(
            "目录包含 {} 个文件，超过上限 {}，请缩小扫描范围",
            files.len(),
            MAX_FILES
        ));
    }

    let base = if root.is_dir() {
        root.clone()
    } else {
        root.parent().unwrap_or(&root).to_path_buf()
    };
    let mut bundle = SourceBundle {
        root: base.clone(),
        ..SourceBundle::default()
    };
    let mut seen_paths = HashSet::new();
    let mut total_chars = 0usize;
    let mut text_parts = Vec::new();

    for file in files {
        let extension = file
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let Some(extension) = extension else {
            continue;
        };
        let relative_path = file.strip_prefix(&base).unwrap_or(&file).to_path_buf();
        if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            if seen_paths.insert(file.clone()) {
                bundle.images.push(ImageSource {
                    path: file,
                    relative_path,
                });
            }
            continue;
        }
        if !TEXT_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }

        if extension == "epub" {
            let content = read_epub_text(&file)?;
            let chars = content.chars().count();
            if total_chars.saturating_add(chars) > MAX_TOTAL_CHARS {
                bundle.warnings.push(format!(
                    "达到总正文上限 {} 个字符，电子书 {} 未读取",
                    MAX_TOTAL_CHARS,
                    relative_path.display()
                ));
                break;
            }
            if !content.trim().is_empty() {
                total_chars += chars;
                bundle.files.push(file);
                text_parts.push(format!(
                    "\n\n<!-- ruiz-source: {} -->\n# 来源：{}\n\n{}",
                    relative_path.display(),
                    relative_path.display(),
                    content.trim()
                ));
            }
            continue;
        }

        let bytes = fs::read(&file)
            .map_err(|error| anyhow!("读取文件 {} 失败: {error}", file.display()))?;
        let content = String::from_utf8_lossy(&bytes).to_string();
        let chars = content.chars().count();
        if chars > MAX_FILE_CHARS {
            bundle.warnings.push(format!(
                "已跳过超过 {} 个字符的文件 {}",
                MAX_FILE_CHARS,
                relative_path.display()
            ));
            continue;
        }
        if total_chars.saturating_add(chars) > MAX_TOTAL_CHARS {
            bundle.warnings.push(format!(
                "达到总正文上限 {} 个字符，后续文件未读取",
                MAX_TOTAL_CHARS
            ));
            break;
        }
        if content.trim().is_empty() {
            continue;
        }
        total_chars += chars;
        bundle.files.push(file);
        text_parts.push(format!(
            "\n\n<!-- ruiz-source: {} -->\n# 来源：{}\n\n{}",
            relative_path.display(),
            relative_path.display(),
            content.trim()
        ));
    }

    bundle.text = text_parts.join("\n");
    bundle.images.sort_by(|left, right| {
        left.relative_path
            .to_string_lossy()
            .cmp(&right.relative_path.to_string_lossy())
    });
    if bundle.is_empty() {
        return Err(anyhow!("路径中没有找到可读取的文本或图片文件"));
    }
    Ok(bundle)
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if path.is_file() {
        files.push(path.to_path_buf());
        if files.len() > MAX_FILES {
            return Err(anyhow!("目录包含超过 {} 个文件，请缩小扫描范围", MAX_FILES));
        }
        return Ok(());
    }
    if !path.is_dir() {
        return Err(anyhow!("路径不是文件或目录: {}", path.display()));
    }
    for entry in
        fs::read_dir(path).map_err(|error| anyhow!("读取目录 {} 失败: {error}", path.display()))?
    {
        let entry = entry?;
        let child = entry.path();
        if should_skip_directory(&child) {
            continue;
        }
        if child.is_dir() {
            collect_files(&child, files)?;
        } else if child.is_file() {
            files.push(child);
        }
    }
    Ok(())
}

fn should_skip_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | ".hg"
                    | ".svn"
                    | "target"
                    | "node_modules"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | "dist"
                    | "build"
            )
        })
}

fn read_epub_text(path: &Path) -> Result<String> {
    let file = fs::File::open(path)
        .map_err(|error| anyhow!("打开电子书 {} 失败: {error}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| anyhow!("读取电子书 {} 失败: {error}", path.display()))?;
    let mut chapters = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_string();
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm")) {
            continue;
        }
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(|error| anyhow!("读取电子书章节 {} 失败: {error}", name))?;
        if !content.trim().is_empty() {
            chapters.push((name, content));
        }
    }
    chapters.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(chapters
        .into_iter()
        .map(|(name, content)| format!("\n\n<!-- epub-chapter: {name} -->\n{content}"))
        .collect::<String>())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::scan_path;

    #[test]
    fn scans_nested_text_and_images_in_stable_order() {
        let root = std::env::temp_dir().join(format!("ruiz-source-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("chapter")).unwrap();
        fs::write(root.join("chapter/b.md"), "第二章").unwrap();
        fs::write(root.join("a.txt"), "第一章").unwrap();
        fs::write(root.join("chapter/figure.png"), [0_u8, 1, 2]).unwrap();

        let bundle = scan_path(&root).unwrap();
        assert_eq!(bundle.files.len(), 2);
        assert_eq!(bundle.images.len(), 1);
        assert!(bundle.text.find("a.txt").unwrap() < bundle.text.find("chapter/b.md").unwrap());
        assert_eq!(
            bundle.images[0].relative_path.to_string_lossy(),
            "chapter/figure.png"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_empty_directory() {
        let root = std::env::temp_dir().join(format!("ruiz-empty-source-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(scan_path(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
