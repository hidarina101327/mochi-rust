//! 定义设备同步配置，并校验仓库、分支和作者信息。
use super::*;

/// 单个隔离设备仓库的配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// 仅供本设备使用、新建的 bare 仓库。
    pub bare_path: PathBuf,
    /// 单独物化出来的工作区。只有通过本 API 提交的已提交快照、
    /// 或从 bare 历史抓取的内容才会写入。
    pub workspace_path: PathBuf,
    /// 稳定的块身份。它在每棵树里都按原样持久化。
    pub block_id: String,
    /// 这个块设备使用的 Git 分支。
    pub branch: String,
    pub author_name: String,
    pub author_email: String,
}

impl DeviceConfig {
    pub fn new(
        bare_path: impl Into<PathBuf>,
        workspace_path: impl Into<PathBuf>,
        block_id: impl Into<String>,
    ) -> Self {
        Self {
            bare_path: bare_path.into(),
            workspace_path: workspace_path.into(),
            block_id: block_id.into(),
            branch: DEFAULT_BRANCH.to_string(),
            author_name: DEFAULT_AUTHOR_NAME.to_string(),
            author_email: DEFAULT_AUTHOR_EMAIL.to_string(),
        }
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    pub fn with_author(mut self, name: impl Into<String>, email: impl Into<String>) -> Self {
        self.author_name = name.into();
        self.author_email = email.into();
        self
    }

    pub(super) fn validate(&self) -> Result<()> {
        ensure!(!self.block_id.is_empty(), "block id must not be empty");
        ensure!(
            !self.block_id.as_bytes().contains(&0),
            "block id contains NUL"
        );
        ensure!(
            !self.author_name.is_empty(),
            "author name must not be empty"
        );
        ensure!(
            !self.author_email.is_empty(),
            "author email must not be empty"
        );
        ensure!(
            !self.author_name.as_bytes().contains(&0),
            "author name contains NUL"
        );
        ensure!(
            !self.author_email.as_bytes().contains(&0),
            "author email contains NUL"
        );
        ensure!(
            git2::Reference::is_valid_name(&format!("refs/heads/{}", self.branch)),
            "invalid branch name: {}",
            self.branch
        );
        ensure!(
            !self.branch.contains('\\'),
            "branch names must use Git's slash form: {}",
            self.branch
        );

        let bare = absolute_for_compare(&self.bare_path)?;
        let workspace = absolute_for_compare(&self.workspace_path)?;
        ensure!(
            bare != workspace,
            "bare repository and workspace must differ"
        );
        ensure!(
            !bare.starts_with(&workspace) && !workspace.starts_with(&bare),
            "bare repository and workspace must not contain one another"
        );
        Ok(())
    }

    pub(super) fn branch_ref(&self) -> String {
        format!("refs/heads/{}", self.branch)
    }

    pub(super) fn tracking_ref(&self) -> String {
        format!("refs/remotes/{TRACKING_REMOTE}/{}", self.branch)
    }
}

pub(super) fn absolute_for_compare(path: &Path) -> Result<PathBuf> {
    let mut path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    path = fs::canonicalize(&path).unwrap_or(path);
    #[cfg(windows)]
    {
        // Rust 的路径相等是词法比较，Windows 打开路径却不区分大小写。
        // 把比较形式统一折算大小写，两种大小写不同的写法才不会让
        // bare 仓库和工作区互相冒名顶替。
        path = PathBuf::from(path.to_string_lossy().to_lowercase());
    }
    Ok(path)
}
