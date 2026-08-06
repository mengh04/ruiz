use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{dynamic_review::QuestionFormat, knowledge::KnowledgeUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlockKind {
    Heading,
    Paragraph,
    List,
    Code,
    Quote,
    Table,
    ThematicBreak,
}

impl ContentBlockKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Heading => "heading",
            Self::Paragraph => "paragraph",
            Self::List => "list",
            Self::Code => "code",
            Self::Quote => "quote",
            Self::Table => "table",
            Self::ThematicBreak => "thematic_break",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "heading" => Ok(Self::Heading),
            "paragraph" => Ok(Self::Paragraph),
            "list" => Ok(Self::List),
            "code" => Ok(Self::Code),
            "quote" => Ok(Self::Quote),
            "table" => Ok(Self::Table),
            "thematic_break" => Ok(Self::ThematicBreak),
            _ => Err(anyhow!("未知正文块类型: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    pub id: Option<i64>,
    pub note_id: i64,
    pub content_hash: String,
    pub local_id: String,
    pub kind: ContentBlockKind,
    pub heading_path: Vec<String>,
    pub source_start: usize,
    pub source_end: usize,
    pub source_text: String,
    pub plain_text: String,
    pub position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRelevance {
    Primary,
    Supporting,
}

impl SourceRelevance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Supporting => "supporting",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitSourceLink {
    pub unit_id: i64,
    pub unit_local_id: String,
    pub block_local_id: String,
    pub relevance: SourceRelevance,
    pub position: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningStepKind {
    Read,
    Checkpoint,
    Recap,
}

impl LearningStepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Checkpoint => "checkpoint",
            Self::Recap => "recap",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "read" => Ok(Self::Read),
            "checkpoint" => Ok(Self::Checkpoint),
            "recap" => Ok(Self::Recap),
            _ => Err(anyhow!("未知学习步骤类型: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningIntent {
    Recall,
    Explain,
    Compare,
    Sequence,
    Predict,
    Diagnose,
    Decide,
}

impl LearningIntent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recall => "recall",
            Self::Explain => "explain",
            Self::Compare => "compare",
            Self::Sequence => "sequence",
            Self::Predict => "predict",
            Self::Diagnose => "diagnose",
            Self::Decide => "decide",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "recall" => Ok(Self::Recall),
            "explain" => Ok(Self::Explain),
            "compare" => Ok(Self::Compare),
            "sequence" => Ok(Self::Sequence),
            "predict" => Ok(Self::Predict),
            "diagnose" => Ok(Self::Diagnose),
            "decide" => Ok(Self::Decide),
            _ => Err(anyhow!("未知学习意图: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningTopic {
    pub id: String,
    pub title: String,
    pub unit_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningStep {
    pub id: Option<i64>,
    pub local_id: String,
    pub topic_id: String,
    pub topic_title: String,
    pub kind: LearningStepKind,
    #[serde(default)]
    pub block_ids: Vec<String>,
    #[serde(default)]
    pub unit_ids: Vec<String>,
    #[serde(default)]
    pub source_step_ids: Vec<String>,
    pub intent: Option<LearningIntent>,
    pub question_format: Option<QuestionFormat>,
    pub reason: Option<String>,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningPlan {
    pub id: Option<i64>,
    pub note_id: i64,
    pub content_hash: String,
    pub plan_version: i64,
    pub summary: String,
    pub estimated_minutes: usize,
    pub generation_mode: String,
    pub topics: Vec<LearningTopic>,
    pub steps: Vec<LearningStep>,
}

#[derive(Debug, Clone)]
pub struct LearningPrompt {
    pub id: Option<i64>,
    pub learning_step_id: i64,
    pub position: usize,
    pub unit_ids: Vec<String>,
    pub format: QuestionFormat,
    pub question: String,
    pub options: Vec<String>,
    pub standard_answer: String,
    pub required_points: Vec<String>,
    pub source_block_ids: Vec<String>,
    pub generation_mode: String,
}

pub fn checkpoint_question_targets(
    step: &LearningStep,
    units: &[KnowledgeUnit],
) -> Vec<Vec<String>> {
    let selected = step
        .unit_ids
        .iter()
        .filter_map(|id| units.iter().find(|unit| &unit.local_id == id))
        .collect::<Vec<_>>();
    let mut targets = Vec::<Vec<&KnowledgeUnit>>::new();
    for unit in selected {
        let can_join_previous = targets.last().is_some_and(|target| {
            target.len() < 2
                && target
                    .first()
                    .is_some_and(|previous| previous.cognitive_action == unit.cognitive_action)
        });
        if can_join_previous {
            targets.last_mut().expect("target exists").push(unit);
        } else {
            targets.push(vec![unit]);
        }
    }
    while targets.len() > 4 {
        let merge_at = targets
            .windows(2)
            .enumerate()
            .min_by_key(|(_, pair)| pair[0].len() + pair[1].len())
            .map(|(index, _)| index)
            .unwrap_or(0);
        let following = targets.remove(merge_at + 1);
        targets[merge_at].extend(following);
    }
    targets
        .into_iter()
        .map(|target| {
            target
                .into_iter()
                .map(|unit| unit.local_id.clone())
                .collect()
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct LearningSession {
    pub id: i64,
    pub plan_id: i64,
    pub current_step_index: usize,
}

/// 依赖固定字节序和常量，跨进程、跨平台稳定，足以用于本地正文版本失效判断。
pub fn content_hash(content: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

pub fn parse_content_blocks(note_id: i64, content: &str) -> Vec<ContentBlock> {
    let hash = content_hash(content);
    let lines = lines_with_offsets(content);
    let mut blocks = Vec::new();
    let mut headings = Vec::<String>::new();
    let mut index = 0;

    while index < lines.len() {
        let (_, _, line) = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        let start_index = index;
        let kind = if fence_marker(line).is_some() {
            let marker = fence_marker(line).unwrap();
            index += 1;
            while index < lines.len() {
                let current = lines[index].2.trim_start();
                index += 1;
                if current.starts_with(marker) {
                    break;
                }
            }
            ContentBlockKind::Code
        } else if heading(line).is_some() {
            index += 1;
            ContentBlockKind::Heading
        } else if is_thematic_break(line) {
            index += 1;
            ContentBlockKind::ThematicBreak
        } else if is_table_start(&lines, index) {
            index += 2;
            while index < lines.len()
                && lines[index].2.contains('|')
                && !lines[index].2.trim().is_empty()
            {
                index += 1;
            }
            ContentBlockKind::Table
        } else if is_list_line(line) {
            index += 1;
            while index < lines.len() {
                let current = lines[index].2;
                if current.trim().is_empty() {
                    break;
                }
                if is_list_line(current) || current.starts_with(' ') || current.starts_with('\t') {
                    index += 1;
                } else {
                    break;
                }
            }
            ContentBlockKind::List
        } else if line.trim_start().starts_with('>') {
            index += 1;
            while index < lines.len() && lines[index].2.trim_start().starts_with('>') {
                index += 1;
            }
            ContentBlockKind::Quote
        } else {
            index += 1;
            while index < lines.len() {
                let current = lines[index].2;
                if current.trim().is_empty()
                    || fence_marker(current).is_some()
                    || heading(current).is_some()
                    || is_thematic_break(current)
                    || is_list_line(current)
                    || current.trim_start().starts_with('>')
                    || is_table_start(&lines, index)
                {
                    break;
                }
                index += 1;
            }
            ContentBlockKind::Paragraph
        };

        let source_start = lines[start_index].0;
        let source_end = lines[index.saturating_sub(1)].1;
        let source_text = content[source_start..source_end]
            .trim_end_matches(['\r', '\n'])
            .to_string();
        let block_heading_path = headings.clone();
        if let Some((level, title)) = heading(&source_text) {
            headings.truncate(level.saturating_sub(1));
            while headings.len() < level.saturating_sub(1) {
                headings.push(String::new());
            }
            headings.push(title.to_string());
        }
        let position = blocks.len();
        blocks.push(ContentBlock {
            id: None,
            note_id,
            content_hash: hash.clone(),
            local_id: format!("B{}", position + 1),
            kind,
            heading_path: block_heading_path,
            source_start,
            source_end,
            plain_text: markdown_to_plain(&source_text),
            source_text,
            position,
        });
    }
    blocks
}

pub fn map_unit_sources(units: &[KnowledgeUnit], blocks: &[ContentBlock]) -> Vec<UnitSourceLink> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let normalized_blocks = blocks
        .iter()
        .map(|block| normalize(&block.plain_text))
        .collect::<Vec<_>>();
    let mut links = Vec::new();
    for unit in units {
        let mut positions = Vec::new();
        for evidence in &unit.evidence {
            let evidence = normalize(evidence);
            if evidence.is_empty() {
                continue;
            }
            positions.extend(
                normalized_blocks
                    .iter()
                    .enumerate()
                    .filter_map(|(index, text)| text.contains(&evidence).then_some(index)),
            );
        }
        positions.sort_unstable();
        positions.dedup();
        if positions.is_empty() {
            let denominator = units.len().max(1);
            positions.push((unit.position * blocks.len() / denominator).min(blocks.len() - 1));
        }
        links.extend(
            positions
                .into_iter()
                .enumerate()
                .map(|(position, block_index)| UnitSourceLink {
                    unit_id: unit.id,
                    unit_local_id: unit.local_id.clone(),
                    block_local_id: blocks[block_index].local_id.clone(),
                    relevance: if position == 0 {
                        SourceRelevance::Primary
                    } else {
                        SourceRelevance::Supporting
                    },
                    position,
                }),
        );
    }
    links
}

pub fn fallback_plan(
    note_id: i64,
    title: &str,
    content_hash: &str,
    blocks: &[ContentBlock],
    units: &[KnowledgeUnit],
    links: &[UnitSourceLink],
) -> LearningPlan {
    let recommended = units
        .iter()
        .filter(|unit| unit.recommended)
        .collect::<Vec<_>>();
    let mut topic_order = Vec::<String>::new();
    for unit in &recommended {
        if !topic_order.contains(&unit.topic) {
            topic_order.push(unit.topic.clone());
        }
    }
    if topic_order.is_empty() {
        topic_order.push("正文".into());
    }
    let topics = topic_order
        .iter()
        .enumerate()
        .map(|(index, topic)| LearningTopic {
            id: format!("T{}", index + 1),
            title: topic.clone(),
            unit_ids: recommended
                .iter()
                .filter(|unit| &unit.topic == topic)
                .map(|unit| unit.local_id.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let topic_id = |title: &str| {
        topics
            .iter()
            .find(|topic| topic.title == title)
            .map(|topic| topic.id.clone())
            .unwrap_or_else(|| topics[0].id.clone())
    };
    let block_positions = blocks
        .iter()
        .map(|block| (block.local_id.as_str(), block.position))
        .collect::<HashMap<_, _>>();
    let mut unit_end = HashMap::<&str, usize>::new();
    for unit in &recommended {
        let position = links
            .iter()
            .filter(|link| link.unit_local_id == unit.local_id)
            .filter_map(|link| block_positions.get(link.block_local_id.as_str()).copied())
            .max()
            .unwrap_or_else(|| unit.position.min(blocks.len().saturating_sub(1)));
        unit_end.insert(unit.local_id.as_str(), position);
    }

    let mut steps = Vec::new();
    let mut block_index = 0;
    while block_index < blocks.len() {
        let start = block_index;
        let mut chars = 0;
        while block_index < blocks.len() && block_index < start + 3 {
            let next = blocks[block_index].source_text.chars().count();
            if block_index > start && chars + next > 2200 {
                break;
            }
            chars += next;
            block_index += 1;
        }
        let introduced = recommended
            .iter()
            .filter(|unit| {
                links.iter().any(|link| {
                    link.unit_local_id == unit.local_id
                        && block_positions
                            .get(link.block_local_id.as_str())
                            .is_some_and(|position| {
                                (*position >= start) && (*position < block_index)
                            })
                })
            })
            .map(|unit| unit.local_id.clone())
            .collect::<Vec<_>>();
        let topic_title = introduced
            .first()
            .and_then(|id| recommended.iter().find(|unit| &unit.local_id == id))
            .map(|unit| unit.topic.clone())
            .unwrap_or_else(|| topics[0].title.clone());
        let read_id = format!("S{}", steps.len() + 1);
        steps.push(LearningStep {
            id: None,
            local_id: read_id.clone(),
            topic_id: topic_id(&topic_title),
            topic_title: topic_title.clone(),
            kind: LearningStepKind::Read,
            block_ids: blocks[start..block_index]
                .iter()
                .map(|block| block.local_id.clone())
                .collect(),
            unit_ids: introduced,
            source_step_ids: Vec::new(),
            intent: None,
            question_format: None,
            reason: None,
            position: steps.len(),
        });
        let due_units = recommended
            .iter()
            .filter(|unit| unit_end.get(unit.local_id.as_str()) == Some(&(block_index - 1)))
            .copied()
            .collect::<Vec<_>>();
        if !due_units.is_empty() {
            let primary = due_units[0];
            steps.push(LearningStep {
                id: None,
                local_id: format!("S{}", steps.len() + 1),
                topic_id: topic_id(&primary.topic),
                topic_title: primary.topic.clone(),
                kind: LearningStepKind::Checkpoint,
                block_ids: Vec::new(),
                unit_ids: due_units.iter().map(|unit| unit.local_id.clone()).collect(),
                source_step_ids: vec![read_id],
                intent: Some(intent_for(primary)),
                question_format: Some(format_for(primary)),
                reason: Some("确认已经理解刚刚阅读的关键内容".into()),
                position: steps.len(),
            });
        }
    }
    for topic in &topics {
        if !topic.unit_ids.is_empty() {
            steps.push(LearningStep {
                id: None,
                local_id: format!("S{}", steps.len() + 1),
                topic_id: topic.id.clone(),
                topic_title: topic.title.clone(),
                kind: LearningStepKind::Recap,
                block_ids: Vec::new(),
                unit_ids: topic.unit_ids.clone(),
                source_step_ids: Vec::new(),
                intent: None,
                question_format: None,
                reason: Some("回顾本主题中需要继续关注的内容".into()),
                position: steps.len(),
            });
        }
    }
    LearningPlan {
        id: None,
        note_id,
        content_hash: content_hash.into(),
        plan_version: 1,
        summary: format!("按原文顺序学习《{title}》，并在关键知识出现后检查理解。"),
        estimated_minutes: ((blocks
            .iter()
            .map(|b| b.plain_text.chars().count())
            .sum::<usize>()
            / 450)
            + recommended.len() * 2)
            .max(1),
        generation_mode: "fallback".into(),
        topics,
        steps,
    }
}

pub fn validate_plan(
    plan: &LearningPlan,
    blocks: &[ContentBlock],
    units: &[KnowledgeUnit],
) -> Result<()> {
    if plan.steps.is_empty() {
        return Err(anyhow!("学习路线没有步骤"));
    }
    let block_ids = blocks
        .iter()
        .map(|b| b.local_id.as_str())
        .collect::<HashSet<_>>();
    let unit_ids = units
        .iter()
        .map(|u| u.local_id.as_str())
        .collect::<HashSet<_>>();
    let topic_ids = plan
        .topics
        .iter()
        .map(|t| t.id.as_str())
        .collect::<HashSet<_>>();
    if topic_ids.len() != plan.topics.len() {
        return Err(anyhow!("学习主题 ID 重复"));
    }
    let mut step_ids = HashSet::new();
    let mut seen_blocks = HashSet::new();
    let mut exposed_units = HashSet::new();
    let mut checked_units = HashSet::new();
    let mut last_block_position = None;
    let positions = blocks
        .iter()
        .map(|b| (b.local_id.as_str(), b.position))
        .collect::<HashMap<_, _>>();
    let mut consecutive_checkpoints = 0;
    for step in &plan.steps {
        if step.local_id.trim().is_empty() || !step_ids.insert(step.local_id.as_str()) {
            return Err(anyhow!("学习步骤 ID 为空或重复"));
        }
        if !topic_ids.contains(step.topic_id.as_str()) {
            return Err(anyhow!("步骤 {} 引用了未知主题", step.local_id));
        }
        if step
            .unit_ids
            .iter()
            .any(|id| !unit_ids.contains(id.as_str()))
        {
            return Err(anyhow!("步骤 {} 引用了未知知识单元", step.local_id));
        }
        match step.kind {
            LearningStepKind::Read => {
                consecutive_checkpoints = 0;
                if step.block_ids.is_empty() {
                    return Err(anyhow!("阅读步骤 {} 没有正文块", step.local_id));
                }
                for block_id in &step.block_ids {
                    if !block_ids.contains(block_id.as_str())
                        || !seen_blocks.insert(block_id.as_str())
                    {
                        return Err(anyhow!("正文块 {block_id} 不存在或被重复阅读"));
                    }
                    let position = positions[block_id.as_str()];
                    if last_block_position.is_some_and(|last| position <= last) {
                        return Err(anyhow!("正文块没有按原文顺序排列"));
                    }
                    last_block_position = Some(position);
                }
                exposed_units.extend(step.unit_ids.iter().map(String::as_str));
            }
            LearningStepKind::Checkpoint => {
                consecutive_checkpoints += 1;
                if consecutive_checkpoints >= 3 || step.unit_ids.is_empty() {
                    return Err(anyhow!("理解检查过密或没有知识单元"));
                }
                if step
                    .unit_ids
                    .iter()
                    .any(|id| !exposed_units.contains(id.as_str()))
                {
                    return Err(anyhow!("理解检查考察了尚未阅读的知识单元"));
                }
                if step
                    .source_step_ids
                    .iter()
                    .any(|id| !step_ids.contains(id.as_str()))
                {
                    return Err(anyhow!("理解检查引用了未来或未知阅读步骤"));
                }
                checked_units.extend(step.unit_ids.iter().map(String::as_str));
            }
            LearningStepKind::Recap => {
                consecutive_checkpoints = 0;
                if step.unit_ids.is_empty() {
                    return Err(anyhow!("主题回顾没有候选知识单元"));
                }
            }
        }
    }
    if seen_blocks.len() != blocks.len() {
        return Err(anyhow!("学习路线没有覆盖全部正文块"));
    }
    if units
        .iter()
        .filter(|unit| unit.recommended)
        .any(|unit| !checked_units.contains(unit.local_id.as_str()))
    {
        return Err(anyhow!("学习路线没有检查全部推荐知识单元"));
    }
    Ok(())
}

fn intent_for(unit: &KnowledgeUnit) -> LearningIntent {
    match unit.cognitive_action.as_str() {
        "explain" => LearningIntent::Explain,
        "compare" => LearningIntent::Compare,
        "sequence" => LearningIntent::Sequence,
        "diagnose" => LearningIntent::Diagnose,
        "decide" => LearningIntent::Decide,
        _ => LearningIntent::Recall,
    }
}

fn format_for(unit: &KnowledgeUnit) -> QuestionFormat {
    match unit.cognitive_action.as_str() {
        "diagnose" | "decide" => QuestionFormat::Application,
        "explain" | "compare" | "sequence" => QuestionFormat::ShortAnswer,
        _ => QuestionFormat::Choice,
    }
}

fn lines_with_offsets(content: &str) -> Vec<(usize, usize, &str)> {
    let mut result = Vec::new();
    let mut start = 0;
    for segment in content.split_inclusive('\n') {
        let end = start + segment.len();
        result.push((start, end, segment.trim_end_matches(['\r', '\n'])));
        start = end;
    }
    if start < content.len() {
        result.push((start, content.len(), &content[start..]));
    }
    result
}

fn fence_marker(line: &str) -> Option<&'static str> {
    let line = line.trim_start();
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_start();
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
        Some((level, line[level + 1..].trim()))
    } else {
        None
    }
}

fn is_list_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with("+ ")
        || line.split_once(". ").is_some_and(|(prefix, _)| {
            !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_digit())
        })
}

fn is_thematic_break(line: &str) -> bool {
    matches!(line.trim(), "---" | "***" | "___")
}

fn is_table_start(lines: &[(usize, usize, &str)], index: usize) -> bool {
    if index + 1 >= lines.len() || !lines[index].2.contains('|') {
        return false;
    }
    let separator = lines[index + 1].2.trim().trim_matches('|');
    !separator.is_empty()
        && separator.split('|').all(|cell| {
            cell.trim().trim_matches(':').chars().all(|ch| ch == '-')
                && cell.trim().trim_matches(':').len() >= 3
        })
}

fn markdown_to_plain(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let line = line.trim();
            let line = line.trim_start_matches('#').trim();
            let line = line.trim_start_matches('>').trim();
            let line = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .or_else(|| line.strip_prefix("+ "))
                .unwrap_or(line);
            line.replace(['`', '*', '_'], "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<String>().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_keeps_fences_lists_and_tables_whole() {
        let content = "# 标题\n\n正文。\n\n- 一\n- 二\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let blocks = parse_content_blocks(1, content);
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[2].kind, ContentBlockKind::List);
        assert_eq!(blocks[3].kind, ContentBlockKind::Code);
        assert_eq!(blocks[4].kind, ContentBlockKind::Table);
        assert!(blocks[3].source_text.contains("fn main"));
    }

    #[test]
    fn content_hash_is_stable_and_changes_with_content() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }

    #[test]
    fn fallback_route_covers_text_and_recommended_units() {
        let content = "# 所有权\n\n值只能有一个所有者。\n\n所有权可以移动。";
        let blocks = parse_content_blocks(1, content);
        let units = vec![KnowledgeUnit {
            id: 1,
            note_id: 1,
            local_id: "K1".into(),
            topic: "所有权".into(),
            objective: "解释所有权移动".into(),
            unit_type: "concept".into(),
            importance: "core".into(),
            stage: "foundation".into(),
            cognitive_action: "explain".into(),
            required_points: vec!["所有权可以移动".into()],
            claim_ids: vec![],
            evidence: vec!["所有权可以移动".into()],
            reason: "核心机制".into(),
            quick: true,
            recommended: true,
            generated: true,
            review_state: Default::default(),
            prerequisite_unit_ids: vec![],
            position: 0,
        }];
        let links = map_unit_sources(&units, &blocks);
        let plan = fallback_plan(1, "Rust", &content_hash(content), &blocks, &units, &links);
        validate_plan(&plan, &blocks, &units).unwrap();
        assert!(
            plan.steps
                .iter()
                .any(|step| step.kind == LearningStepKind::Checkpoint)
        );
        let checkpoint = plan
            .steps
            .iter()
            .find(|step| step.kind == LearningStepKind::Checkpoint)
            .unwrap();
        assert_eq!(checkpoint_question_targets(checkpoint, &units).len(), 1);

        let actions = [
            "compare", "explain", "explain", "explain", "diagnose", "explain", "explain",
        ];
        let expanded_units = actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let mut unit = units[0].clone();
                unit.local_id = format!("K{}", index + 1);
                unit.cognitive_action = (*action).into();
                unit
            })
            .collect::<Vec<_>>();
        let mut broad_checkpoint = checkpoint.clone();
        broad_checkpoint.unit_ids = expanded_units
            .iter()
            .map(|unit| unit.local_id.clone())
            .collect();
        let targets = checkpoint_question_targets(&broad_checkpoint, &expanded_units);
        assert_eq!(targets.len(), 4);
        assert_eq!(targets.iter().flatten().count(), expanded_units.len());
        assert!(targets.iter().all(|target| target.len() <= 2));
    }
}
