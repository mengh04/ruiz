/// 智能导入的主要阶段，用于在界面中展示当前进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Preparing,
    Cleaning,
    Organizing,
    Extracting,
    Reconciling,
    Generating,
    Saving,
}

impl ImportStage {
    pub const TOTAL: usize = 5;

    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "准备导入",
            Self::Cleaning => "清洗网页噪声",
            Self::Organizing => "归并并命名材料",
            Self::Extracting => "提取知识单元",
            Self::Reconciling => "整理知识蓝图",
            Self::Generating => "生成推荐卡片",
            Self::Saving => "保存到资料库",
        }
    }

    pub fn position(self) -> usize {
        match self {
            Self::Preparing => 0,
            Self::Cleaning => 1,
            Self::Organizing => 2,
            Self::Extracting => 3,
            Self::Reconciling => 3,
            Self::Generating => 4,
            Self::Saving => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProgress {
    pub stage: ImportStage,
    pub detail: String,
}

impl ImportProgress {
    pub fn preparing() -> Self {
        Self {
            stage: ImportStage::Preparing,
            detail: "正在检查输入并准备完整上下文请求".into(),
        }
    }

    pub fn stage(stage: ImportStage, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }
}

pub type ImportProgressReporter = dyn Fn(ImportProgress) + Send + Sync;
