use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{
    client::{ChatClient, StreamEvent},
    progress::{ImportCancellation, ImportEvent, ImportEventReporter, ImportProgress, ImportStage},
    prompts::{IMPORT_CLEAN_SYSTEM, IMPORT_ORGANIZE_SYSTEM},
    text::{normalize_source, preview},
};

const MAX_MATERIALS: usize = 20;

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
    progress(ImportEvent::Stage(ImportProgress::stage(
        ImportStage::Cleaning,
        format!(
            "正在一次性清洗整篇材料（{} 个字符），移除导航、广告和重复目录",
            source.chars().count()
        ),
    )));
    let input = serde_json::json!({ "raw_source": source });
    let report_stream = |event| {
        if let StreamEvent::Thinking(text) = event {
            progress(ImportEvent::Thinking(text));
        }
    };
    let value = client
        .chat_json_stream_for(
            "import.clean",
            IMPORT_CLEAN_SYSTEM,
            &input.to_string(),
            &report_stream,
            Some(cancellation),
        )
        .await?;
    let response: CleanResponse =
        serde_json::from_value(value).map_err(|error| anyhow!("材料清洗响应格式不对: {error}"))?;
    let mut stored = Vec::new();
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
            source_order: index,
        });
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
            content,
            raw_content: source.clone(),
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
