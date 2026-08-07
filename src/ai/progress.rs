use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use tokio::sync::Notify;

/// 智能导入的主要阶段，用于在界面中展示当前进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Preparing,
    DescribingImages,
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
            Self::DescribingImages => "识别本地图片",
            Self::Cleaning => "清洗网页噪声",
            Self::Organizing => "归并并命名材料",
            Self::Extracting => "提取知识单元",
            Self::Reconciling => "整理知识蓝图",
            Self::Generating => "准备复习备用题",
            Self::Saving => "保存到资料库",
        }
    }

    pub fn position(self) -> usize {
        match self {
            Self::Preparing => 0,
            Self::DescribingImages => 1,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportEvent {
    Stage(ImportProgress),
    Thinking(String),
    Answer(String),
}

pub type ImportEventReporter = dyn Fn(ImportEvent) + Send + Sync;

#[derive(Clone, Default)]
pub struct ImportCancellation {
    state: Arc<AtomicU8>,
    notify: Arc<Notify>,
}

impl ImportCancellation {
    const ACTIVE: u8 = 0;
    const CANCELLED: u8 = 1;
    const PERSISTING: u8 = 2;

    pub fn cancel(&self) -> bool {
        if self
            .state
            .compare_exchange(
                Self::ACTIVE,
                Self::CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.notify.notify_waiters();
            true
        } else {
            false
        }
    }

    pub fn can_cancel(&self) -> bool {
        self.state.load(Ordering::Acquire) == Self::ACTIVE
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == Self::CANCELLED
    }

    pub fn ensure_active(&self) -> anyhow::Result<()> {
        if self.is_cancelled() {
            Err(anyhow::anyhow!("导入任务已取消"))
        } else {
            Ok(())
        }
    }

    pub fn begin_persistence(&self) -> anyhow::Result<()> {
        match self.state.compare_exchange(
            Self::ACTIVE,
            Self::PERSISTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(Self::PERSISTING) => Ok(()),
            Err(Self::CANCELLED) => Err(anyhow::anyhow!("导入任务已取消")),
            Err(_) => Err(anyhow::anyhow!("导入任务状态无效")),
        }
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::ImportCancellation;

    #[test]
    fn cancellation_succeeds_once_and_blocks_persistence() {
        let cancellation = ImportCancellation::default();

        assert!(cancellation.can_cancel());
        assert!(cancellation.cancel());
        assert!(!cancellation.can_cancel());
        assert!(cancellation.is_cancelled());
        assert!(!cancellation.cancel());
        assert!(cancellation.ensure_active().is_err());
        assert!(cancellation.begin_persistence().is_err());
    }

    #[test]
    fn persistence_lock_makes_cancellation_ineffective() {
        let cancellation = ImportCancellation::default();

        cancellation.begin_persistence().unwrap();
        assert!(!cancellation.can_cancel());
        assert!(!cancellation.cancel());
        assert!(!cancellation.is_cancelled());
        assert!(cancellation.ensure_active().is_ok());
        assert!(cancellation.begin_persistence().is_ok());
    }
}
