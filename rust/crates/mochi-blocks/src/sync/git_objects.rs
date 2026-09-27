//! 管理 Git 裸仓库、对象引用和工作区仓库关系。
use super::*;

pub(super) fn validate_branch(branch: &str) -> Result<()> {
    ensure!(
        git2::Reference::is_valid_name(&format!("refs/heads/{branch}")),
        "invalid branch name: {branch}"
    );
    ensure!(!branch.contains('\\'), "branch names must use slash form");
    Ok(())
}

pub(super) fn init_new_bare(path: &Path) -> Result<Repository> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                !metadata.file_type().is_symlink(),
                "bare repository path must not be a symlink"
            );
            ensure!(metadata.is_dir(), "bare repository path is not a directory");
            ensure!(
                fs::read_dir(path)?.next().is_none(),
                "refusing to initialise a non-empty explicit bare repository path"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
        }
        Err(error) => return Err(error.into()),
    }
    Repository::init_bare(path)
        .with_context(|| format!("initialise bare repository {}", path.display()))
}

pub(super) fn ensure_workspace_root(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                !metadata.file_type().is_symlink(),
                "workspace root must not be a symlink"
            );
            ensure!(metadata.is_dir(), "workspace path is not a directory");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)
                .with_context(|| format!("create explicit workspace {}", path.display()))?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(super) fn ensure_distinct_repositories(local: &Path, remote: &Path) -> Result<()> {
    let local = absolute_for_compare(local)?;
    let remote = absolute_for_compare(remote)?;
    ensure!(
        local != remote,
        "local and remote bare repositories must differ"
    );
    Ok(())
}

pub(super) fn ref_target(repo: &Repository, name: &str) -> Result<Option<Oid>> {
    match repo.find_reference(name) {
        Ok(reference) => Ok(reference.target()),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn update_ref_if_current(
    repo: &Repository,
    name: &str,
    expected: Option<Oid>,
    new_target: Oid,
    message: &str,
) -> Result<()> {
    let mut transaction = repo.transaction()?;
    transaction.lock_ref(name)?;
    let actual = ref_target(repo, name)?;
    ensure!(
        actual == expected,
        "reference changed while committing; no head was updated"
    );
    let signature = transport_signature()?;
    transaction.set_target(name, new_target, Some(&signature), message)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn set_tracking_ref(repo: &Repository, name: &str, target: Oid) -> Result<()> {
    // tracking ref 只是本地观察结果。完整的 fetch/push 响应收到后才替换；
    // 设备分支本身绝不被强推。
    let mut transaction = repo.transaction()?;
    transaction.lock_ref(name)?;
    let signature = transport_signature()?;
    transaction.set_target(name, target, Some(&signature), "mochi blocks tracking")?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn transport_signature() -> Result<Signature<'static>> {
    Ok(Signature::now(DEFAULT_AUTHOR_NAME, DEFAULT_AUTHOR_EMAIL)?)
}

pub(super) fn block_id_comment_bytes(block_id: &str) -> Vec<u8> {
    let mut bytes = BLOCK_ID_COMMENT_HEADER.to_vec();
    bytes.extend_from_slice(block_id.as_bytes());
    bytes
}

pub(super) fn parse_block_id(bytes: &[u8]) -> Result<String> {
    let id = bytes
        .strip_prefix(BLOCK_ID_COMMENT_HEADER)
        .context("invalid Mochi block id comment file")?;
    Ok(std::str::from_utf8(id)
        .context("block id comment file is not UTF-8")?
        .to_string())
}

pub(super) enum TreeNode {
    Blob(Vec<u8>),
    Tree(BTreeMap<String, TreeNode>),
}

pub(super) type CommittedFiles = BTreeMap<String, Vec<u8>>;

pub(super) type MergeMaps = (CommittedFiles, Vec<ConflictFile>);

pub(super) fn map_to_tree(repo: &Repository, files: &BTreeMap<String, Vec<u8>>) -> Result<Oid> {
    validate_file_map(files, true)?;
    let mut root = BTreeMap::new();
    for (path, bytes) in files {
        let components: Vec<&str> = path.split('/').collect();
        insert_tree_node(&mut root, &components, bytes.clone())?;
    }
    write_tree_node(repo, &root)
}

fn insert_tree_node(
    tree: &mut BTreeMap<String, TreeNode>,
    components: &[&str],
    bytes: Vec<u8>,
) -> Result<()> {
    let component = components.first().context("empty tree path")?;
    if components.len() == 1 {
        match tree.get(*component) {
            Some(TreeNode::Tree(_)) => {
                bail!("file and directory path collision at {component}")
            }
            _ => {
                tree.insert((*component).to_string(), TreeNode::Blob(bytes));
                return Ok(());
            }
        }
    }

    let entry = tree
        .entry((*component).to_string())
        .or_insert_with(|| TreeNode::Tree(BTreeMap::new()));
    match entry {
        TreeNode::Blob(_) => bail!("file and directory path collision at {component}"),
        TreeNode::Tree(children) => insert_tree_node(children, &components[1..], bytes),
    }
}

fn write_tree_node(repo: &Repository, tree: &BTreeMap<String, TreeNode>) -> Result<Oid> {
    let mut builder = repo.treebuilder(None)?;
    for (name, node) in tree {
        let (oid, mode) = match node {
            TreeNode::Blob(bytes) => (repo.blob(bytes)?, 0o100644),
            TreeNode::Tree(children) => (write_tree_node(repo, children)?, 0o040000),
        };
        builder.insert(name, oid, mode)?;
    }
    Ok(builder.write()?)
}

pub(super) fn commit_files(repo: &Repository, commit_id: Oid) -> Result<BTreeMap<String, Vec<u8>>> {
    let commit = repo.find_commit(commit_id)?;
    let tree = commit.tree()?;
    let mut files = BTreeMap::new();
    read_tree(repo, &tree, "", &mut files)?;
    validate_file_map(&files, true)?;
    Ok(files)
}

fn read_tree(
    repo: &Repository,
    tree: &Tree<'_>,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for entry in tree {
        let name = std::str::from_utf8(entry.name_bytes())
            .context("Git tree contains a non-UTF-8 path")?;
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if path != BLOCK_ID_COMMENT_PATH {
            validate_snapshot_path(&path)?;
        }
        match entry.kind() {
            Some(ObjectType::Tree) => {
                let child = repo.find_tree(entry.id())?;
                read_tree(repo, &child, &path, files)?;
            }
            Some(ObjectType::Blob) => {
                ensure!(
                    entry.filemode_raw() != 0o120000,
                    "symlink entries are not accepted in block history: {path}"
                );
                let blob = repo.find_blob(entry.id())?;
                files.insert(path, blob.content().to_vec());
            }
            other => bail!("unsupported Git tree entry at {path}: {other:?}"),
        }
    }
    Ok(())
}

pub(super) fn copy_reachable_objects(src: &Repository, dst: &Repository, root: Oid) -> Result<()> {
    let mut seen = HashSet::new();
    copy_object(src, dst, root, &mut seen)
}

fn copy_object(
    src: &Repository,
    dst: &Repository,
    oid: Oid,
    seen: &mut HashSet<Oid>,
) -> Result<()> {
    if !seen.insert(oid) {
        return Ok(());
    }
    let src_odb = src.odb()?;
    let object = src_odb.read(oid)?;
    let kind = object.kind();
    dst.odb()?.write(kind, object.data())?;
    drop(object);

    match kind {
        ObjectType::Commit => {
            let commit = src.find_commit(oid)?;
            copy_object(src, dst, commit.tree_id(), seen)?;
            for parent in commit.parents() {
                copy_object(src, dst, parent.id(), seen)?;
            }
        }
        ObjectType::Tree => {
            let tree = src.find_tree(oid)?;
            for entry in &tree {
                copy_object(src, dst, entry.id(), seen)?;
            }
        }
        ObjectType::Blob => {}
        other => bail!("unsupported object type in block history: {other:?}"),
    }
    Ok(())
}
