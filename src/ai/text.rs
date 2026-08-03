use anyhow::{Result, anyhow};

pub const MAX_IMPORT_CHARS: usize = 300_000;

pub fn normalize_source(raw: &str) -> Result<String> {
    let normalized = raw
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "");
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return Err(anyhow!("导入内容不能为空"));
    }
    let count = normalized.chars().count();
    if count > MAX_IMPORT_CHARS {
        return Err(anyhow!(
            "单次导入最多支持 {MAX_IMPORT_CHARS} 个字符，当前为 {count} 个字符"
        ));
    }
    Ok(normalized.to_string())
}

pub fn preview(text: &str, limit: usize) -> String {
    let mut value = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::normalize_source;

    #[test]
    fn rejects_empty_source() {
        assert!(normalize_source("\r\n\0 ").is_err());
    }
}
