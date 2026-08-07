use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{
    client::{ChatClient, StreamEvent},
    progress::{ImportCancellation, ImportEvent, ImportEventReporter, ImportProgress, ImportStage},
    prompts::{IMPORT_CLEAN_SYSTEM, IMPORT_ORGANIZE_SYSTEM},
    text::{normalize_source, preview},
};

const MAX_MATERIALS: usize = 200;
const MAX_CLEAN_CHUNK_CHARS: usize = 80_000;

#[derive(Debug, Clone)]
pub struct ImportedMaterial {
    pub title: String,
    pub content: String,
    pub raw_content: String,
    pub summary: String,
    pub document_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CleanFragment {
    material_key: String,
    title_hint: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct CleanResponse {
    fragments: Vec<CleanFragment>,
}

#[derive(Debug, Clone)]
struct StoredFragment {
    id: String,
    key: String,
    title_hint: String,
    content: String,
    source_order: usize,
}

#[derive(Debug, Serialize)]
struct FragmentMetadata {
    fragment_id: String,
    material_key: String,
    title_hint: String,
    source_order: usize,
    character_count: usize,
    start_preview: String,
    end_preview: String,
}

#[derive(Debug, Deserialize)]
struct OrganizedResponse {
    materials: Vec<OrganizedMaterial>,
}

#[derive(Debug, Deserialize)]
struct OrganizedMaterial {
    title: String,
    summary: String,
    document_type: String,
    fragment_ids: Vec<String>,
}

pub async fn import_materials(
    client: &ChatClient,
    raw: &str,
    progress: &ImportEventReporter,
    cancellation: &ImportCancellation,
) -> Result<Vec<ImportedMaterial>> {
    cancellation.ensure_active()?;
    let source = normalize_source(raw)?;
    let chunks = split_clean_chunks(&source);
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Cleaning,
        format!(
            "正在分段清洗材料（{} 个字符，共 {} 段），移除导航、广告和重复目录",
            source.chars().count(),
            chunks.len()
        ),
    )));
    let report_stream = |event| match event {
        StreamEvent::Thinking(text) => progress(ImportEvent::Thinking(text)),
        StreamEvent::Content(text) => progress(ImportEvent::Answer(text)),
    };
    let mut stored = Vec::new();
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        cancellation.ensure_active()?;
        let input = serde_json::json!({
            "chunk_index": chunk_index + 1,
            "chunk_count": chunks.len(),
            "raw_source": chunk,
        });
        let value = client
            .chat_json_stream_for(
                "import.clean",
                IMPORT_CLEAN_SYSTEM,
                &input.to_string(),
                &report_stream,
                Some(cancellation),
            )
            .await?;
        let response: CleanResponse = serde_json::from_value(value).map_err(|error| {
            anyhow!("材料清洗响应格式不对（第 {} 段）: {error}", chunk_index + 1)
        })?;
        for (index, fragment) in response.fragments.into_iter().enumerate() {
            let key = fragment.material_key.trim();
            let content = fragment.content.trim();
            if key.is_empty() || content.is_empty() {
                continue;
            }
            stored.push(StoredFragment {
                id: format!("fragment-{}", stored.len() + 1),
                key: key.chars().take(80).collect(),
                title_hint: fragment.title_hint.trim().chars().take(100).collect(),
                content: content.to_string(),
                source_order: chunk_index * MAX_CLEAN_CHUNK_CHARS + index,
            });
        }
    }

    cancellation.ensure_active()?;
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Organizing,
        format!(
            "正在归并 {} 个有效片段，并自动生成材料标题和摘要",
            stored.len()
        ),
    )));
    if stored.is_empty() {
        return Err(anyhow!("AI 没有从输入中识别出可学习的正文内容"));
    }

    let metadata = stored
        .iter()
        .map(|fragment| FragmentMetadata {
            fragment_id: fragment.id.clone(),
            material_key: fragment.key.clone(),
            title_hint: fragment.title_hint.clone(),
            source_order: fragment.source_order,
            character_count: fragment.content.chars().count(),
            start_preview: preview(&fragment.content, 240),
            end_preview: fragment
                .content
                .chars()
                .rev()
                .take(240)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
        })
        .collect::<Vec<_>>();
    let input = serde_json::json!({ "fragments": metadata });
    let value = client
        .chat_json_stream_for(
            "import.organize",
            IMPORT_ORGANIZE_SYSTEM,
            &input.to_string(),
            &report_stream,
            Some(cancellation),
        )
        .await?;
    let organized: OrganizedResponse =
        serde_json::from_value(value).map_err(|error| anyhow!("材料整理响应格式不对: {error}"))?;
    validate_organization(&organized, &stored)?;

    let by_id = stored
        .into_iter()
        .map(|fragment| (fragment.id.clone(), fragment))
        .collect::<HashMap<_, _>>();
    let mut materials = Vec::new();
    for material in organized.materials {
        let mut fragments = material
            .fragment_ids
            .iter()
            .filter_map(|id| by_id.get(id))
            .collect::<Vec<_>>();
        fragments.sort_by_key(|fragment| fragment.source_order);
        let content = fragments
            .iter()
            .map(|fragment| fragment.content.trim())
            .collect::<Vec<_>>()
            .join("\n\n");
        if content.trim().is_empty() {
            continue;
        }
        materials.push(ImportedMaterial {
            title: non_empty(material.title, "未命名学习材料", 120),
            raw_content: content.clone(),
            content,
            summary: material.summary.trim().chars().take(500).collect(),
            document_type: non_empty(material.document_type, "mixed", 40),
        });
    }
    if materials.is_empty() {
        return Err(anyhow!("AI 整理后没有留下有效材料"));
    }
    Ok(materials)
}

fn validate_organization(response: &OrganizedResponse, fragments: &[StoredFragment]) -> Result<()> {
    if response.materials.is_empty() || response.materials.len() > MAX_MATERIALS {
        return Err(anyhow!("AI 返回的材料数量不合理"));
    }
    let expected = fragments
        .iter()
        .map(|fragment| fragment.id.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for material in &response.materials {
        if material.title.trim().is_empty() || material.fragment_ids.is_empty() {
            return Err(anyhow!("AI 返回了空标题或空材料"));
        }
        for id in &material.fragment_ids {
            if !expected.contains(id.as_str()) {
                return Err(anyhow!("AI 引用了不存在的材料片段: {id}"));
            }
            if !seen.insert(id.as_str()) {
                return Err(anyhow!("AI 重复使用了材料片段: {id}"));
            }
        }
    }
    if seen != expected {
        return Err(anyhow!("AI 没有完整归类所有清洗后的材料片段"));
    }
    Ok(())
}

fn non_empty(value: String, fallback: &str, max: usize) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

fn split_clean_chunks(source: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0;

    for paragraph in source
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
    {
        let paragraph_chars = paragraph.chars().count();
        if paragraph_chars > MAX_CLEAN_CHUNK_CHARS {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_chars = 0;
            }
            let mut piece = String::new();
            for character in paragraph.chars() {
                piece.push(character);
                if piece.chars().count() >= MAX_CLEAN_CHUNK_CHARS {
                    chunks.push(std::mem::take(&mut piece));
                }
            }
            if !piece.is_empty() {
                chunks.push(piece);
            }
            continue;
        }

        let separator_chars = if current.is_empty() { 0 } else { 2 };
        if current_chars + separator_chars + paragraph_chars > MAX_CLEAN_CHUNK_CHARS
            && !current.is_empty()
        {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
            current_chars += 2;
        }
        current.push_str(paragraph);
        current_chars += paragraph_chars;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::split_clean_chunks;

    #[test]
    fn long_sources_are_split_without_dropping_paragraphs() {
        let source = format!("{}\n\n{}", "甲".repeat(50_000), "乙".repeat(50_000));
        let chunks = split_clean_chunks(&source);
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks.concat().chars().filter(|c| *c == '甲').count(),
            50_000
        );
        assert_eq!(
            chunks.concat().chars().filter(|c| *c == '乙').count(),
            50_000
        );
    }
}
