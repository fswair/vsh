use std::collections::BTreeMap;

use monty_types::{
    DictPairs, ExcType, MkdirCallArgs, MontyException, MontyObject, MontyPath, OsFunctionCall,
    PathBytesDataArgs, PathStringDataArgs, RenameCallArgs,
};
use vsh_policy::AccessKind;
use vsh_types::{NodeKind, NodeState, VPath};
use vsh_vfs::{VfsError, VirtualFs};

use super::{
    Budget, CallFailure, InProcessConfig, already_exists, authorize_path, check_path_bytes,
    classify_vfs, dispatch_call, is_directory_exception, map_authorized_path, map_call_path,
    not_directory, permission_denied, vfs,
};

/// Stable VSH functions injected into every Monty program.
pub const MONTY_VSH_TOOL_NAMES: &[&str] = &[
    "vsh_read",
    "vsh_write",
    "vsh_list",
    "vsh_mkdir",
    "vsh_remove",
    "vsh_move",
    "vsh_copy",
    "vsh_glob",
    "vsh_search",
    "vsh_patch",
];

const TOOL_SPECS: &[(&str, &str)] = &[
    (
        "vsh_read",
        "vsh_read(path, binary=False) -> str | bytes\nRead from the active VSH virtual snapshot.",
    ),
    (
        "vsh_write",
        "vsh_write(path, data, append=False) -> int\nWrite str or bytes into the active VSH overlay.",
    ),
    (
        "vsh_list",
        "vsh_list(path='/workspace') -> list[Path]\nList one directory in the active VSH snapshot.",
    ),
    (
        "vsh_mkdir",
        "vsh_mkdir(path, parents=True, exist_ok=True) -> None\nCreate a directory in the active VSH overlay.",
    ),
    (
        "vsh_remove",
        "vsh_remove(path, recursive=False, missing_ok=False) -> None\nRemove a file or directory from the active VSH overlay.",
    ),
    (
        "vsh_move",
        "vsh_move(source, destination) -> Path\nMove a virtual path without touching the host directly.",
    ),
    (
        "vsh_copy",
        "vsh_copy(source, destination, recursive=False, overwrite=False) -> Path\nCopy files or directory trees inside the active VSH overlay.",
    ),
    (
        "vsh_glob",
        "vsh_glob(pattern, path='/workspace', max_results=1000) -> list[Path]\nMatch *, ?, and ** against the active VSH snapshot.",
    ),
    (
        "vsh_search",
        "vsh_search(query, path='/workspace', case_sensitive=True, max_results=100) -> list[dict]\nSearch UTF-8 files in the active VSH snapshot.",
    ),
    (
        "vsh_patch",
        "vsh_patch(path, old, new, count=1) -> int\nReplace exact UTF-8 text in one active virtual file.",
    ),
];

pub(super) fn inputs() -> (Vec<String>, Vec<MontyObject>) {
    let mut names = Vec::with_capacity(TOOL_SPECS.len());
    let mut values = Vec::with_capacity(TOOL_SPECS.len());
    for (name, docstring) in TOOL_SPECS {
        names.push((*name).to_owned());
        values.push(MontyObject::Function {
            name: (*name).to_owned(),
            docstring: Some((*docstring).to_owned()),
        });
    }
    (names, values)
}

pub(super) fn is_tool(name: &str) -> bool {
    MONTY_VSH_TOOL_NAMES.contains(&name)
}

pub(super) fn dispatch(
    name: &str,
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    match name {
        "vsh_read" => read(args, kwargs, filesystem, config, budget),
        "vsh_write" => write(args, kwargs, filesystem, config, budget),
        "vsh_list" => list(args, kwargs, filesystem, config, budget),
        "vsh_mkdir" => make_directory(args, kwargs, filesystem, config, budget),
        "vsh_remove" => remove(args, kwargs, filesystem, config, budget),
        "vsh_move" => move_path(args, kwargs, filesystem, config, budget),
        "vsh_copy" => copy(args, kwargs, filesystem, config, budget),
        "vsh_glob" => glob(args, kwargs, filesystem, config, budget),
        "vsh_search" => search(args, kwargs, filesystem, config, budget),
        "vsh_patch" => patch(args, kwargs, filesystem, config, budget),
        _ => Err(runtime_error(format!("unknown VSH Monty tool: {name}"))),
    }
}

fn read(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind("vsh_read", args, kwargs, &["path", "binary"])?;
    let path = args.required_path("path")?;
    let binary = args.optional_bool("binary", false)?;
    let call = if binary {
        OsFunctionCall::ReadBytes(MontyPath::from(path))
    } else {
        OsFunctionCall::ReadText(MontyPath::from(path))
    };
    dispatch_call(&call, filesystem, config, budget)
}

fn write(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind("vsh_write", args, kwargs, &["path", "data", "append"])?;
    let path = args.required_path("path")?;
    let data = args.required("data")?;
    let append = args.optional_bool("append", false)?;
    let call = match (data, append) {
        (MontyObject::String(data), false) => OsFunctionCall::WriteText(PathStringDataArgs {
            path: MontyPath::from(path),
            data: data.clone(),
        }),
        (MontyObject::String(data), true) => OsFunctionCall::AppendText(PathStringDataArgs {
            path: MontyPath::from(path),
            data: data.clone(),
        }),
        (MontyObject::Bytes(data), false) => OsFunctionCall::WriteBytes(PathBytesDataArgs {
            path: MontyPath::from(path),
            data: data.clone(),
        }),
        (MontyObject::Bytes(data), true) => OsFunctionCall::AppendBytes(PathBytesDataArgs {
            path: MontyPath::from(path),
            data: data.clone(),
        }),
        (value, _) => {
            return Err(type_error(format!(
                "vsh_write() argument 'data' must be str or bytes, not {}",
                value.type_name()
            )));
        }
    };
    dispatch_call(&call, filesystem, config, budget)
}

fn list(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind("vsh_list", args, kwargs, &["path"])?;
    let path = args.optional_path("path", config.virtual_root.as_str())?;
    dispatch_call(
        &OsFunctionCall::Iterdir(MontyPath::from(path)),
        filesystem,
        config,
        budget,
    )
}

fn make_directory(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind("vsh_mkdir", args, kwargs, &["path", "parents", "exist_ok"])?;
    let call = OsFunctionCall::Mkdir(MkdirCallArgs {
        path: MontyPath::from(args.required_path("path")?),
        parents: args.optional_bool("parents", true)?,
        exist_ok: args.optional_bool("exist_ok", true)?,
    });
    dispatch_call(&call, filesystem, config, budget)
}

fn move_path(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind("vsh_move", args, kwargs, &["source", "destination"])?;
    let source = args.required_path("source")?;
    let destination = args.required_path("destination")?;
    let call = OsFunctionCall::Rename(RenameCallArgs {
        src: MontyPath::from(source),
        dst: MontyPath::from(destination),
    });
    dispatch_call(&call, filesystem, config, budget)?;
    let mapped = map_call_path(&call, destination, true, config)?;
    Ok(MontyObject::Path(config.virtual_root.present(&mapped)))
}

fn remove(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind(
        "vsh_remove",
        args,
        kwargs,
        &["path", "recursive", "missing_ok"],
    )?;
    let raw = args.required_path("path")?;
    let recursive = args.optional_bool("recursive", false)?;
    let missing_ok = args.optional_bool("missing_ok", false)?;
    let marker = OsFunctionCall::Unlink(MontyPath::from(raw));
    let path = map_authorized_path(&marker, raw, false, config, &[AccessKind::Delete])?;
    let state = match filesystem.metadata(&path) {
        Ok(state) => state,
        Err(VfsError::NotFound { .. }) if missing_ok => return Ok(MontyObject::None),
        Err(error) => return Err(classify_vfs(error, raw)),
    };
    match state.kind() {
        NodeKind::File | NodeKind::Symlink => vfs(filesystem.unlink(&path), raw)?,
        NodeKind::Directory if recursive => {
            preflight_recursive_delete(&path, filesystem, config, budget)?;
            vfs(filesystem.remove_tree(&path), raw)?;
        }
        NodeKind::Directory => vfs(filesystem.rmdir(&path), raw)?,
    }
    Ok(MontyObject::None)
}

fn preflight_recursive_delete(
    root: &VPath,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<(), CallFailure> {
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        authorize_path(
            config,
            &directory,
            &[AccessKind::Delete, AccessKind::DirectoryRead],
        )?;
        let children = vfs(filesystem.read_dir(&directory), directory.as_str())?;
        budget.charge_directory_entries(children.len())?;
        for child in children {
            authorize_path(config, &child, &[AccessKind::Delete])?;
            let state = vfs(filesystem.metadata(&child), child.as_str())?;
            if state.kind() == NodeKind::Directory {
                pending.push(child);
            }
        }
    }
    Ok(())
}

fn copy(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind(
        "vsh_copy",
        args,
        kwargs,
        &["source", "destination", "recursive", "overwrite"],
    )?;
    let source_raw = args.required_path("source")?;
    let destination_raw = args.required_path("destination")?;
    let recursive = args.optional_bool("recursive", false)?;
    let overwrite = args.optional_bool("overwrite", false)?;
    let marker = OsFunctionCall::Rename(RenameCallArgs {
        src: MontyPath::from(source_raw),
        dst: MontyPath::from(destination_raw),
    });
    let source = map_authorized_path(
        &marker,
        source_raw,
        false,
        config,
        &[AccessKind::MetadataRead],
    )?;
    let destination = map_authorized_path(
        &marker,
        destination_raw,
        true,
        config,
        &[AccessKind::Create, AccessKind::Modify],
    )?;
    if destination == source || destination.is_within(&source) {
        return Err(value_error(
            "vsh_copy() destination cannot be inside its source".to_owned(),
        ));
    }
    require_destination_parent(&destination, filesystem, destination_raw)?;
    let source_state = vfs(filesystem.metadata(&source), source_raw)?;
    match source_state.kind() {
        NodeKind::File => copy_file(
            &source,
            &destination,
            source_state,
            overwrite,
            filesystem,
            config,
            budget,
        )?,
        NodeKind::Directory if recursive => copy_tree(
            &source,
            &destination,
            source_state,
            overwrite,
            filesystem,
            config,
            budget,
        )?,
        NodeKind::Directory => {
            return Err(value_error(
                "vsh_copy() source is a directory; pass recursive=True".to_owned(),
            ));
        }
        NodeKind::Symlink => {
            return Err(value_error(
                "vsh_copy() does not copy symbolic links".to_owned(),
            ));
        }
    }
    Ok(MontyObject::Path(config.virtual_root.present(&destination)))
}

fn require_destination_parent(
    destination: &VPath,
    filesystem: &mut VirtualFs,
    raw: &str,
) -> Result<(), CallFailure> {
    let Some(parent) = destination.parent() else {
        return Err(CallFailure::Python(permission_denied(raw)));
    };
    let state = vfs(filesystem.metadata(&parent), raw)?;
    if state.kind() != NodeKind::Directory {
        return Err(not_directory(raw));
    }
    Ok(())
}

fn copy_file(
    source: &VPath,
    destination: &VPath,
    source_state: NodeState,
    overwrite: bool,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<(), CallFailure> {
    match filesystem.metadata(destination) {
        Ok(_) if !overwrite => return Err(already_exists(destination.as_str())),
        Ok(state) if state.kind() == NodeKind::Directory => {
            return Err(CallFailure::Python(is_directory_exception(
                destination.as_str(),
            )));
        }
        Ok(_) | Err(VfsError::NotFound { .. }) => {}
        Err(error) => return Err(classify_vfs(error, destination.as_str())),
    }
    authorize_path(config, source, &[AccessKind::ContentRead])?;
    budget.charge_read(source_state.size())?;
    let bytes = vfs(filesystem.read(source), source.as_str())?;
    budget.charge_write(bytes.len())?;
    vfs(filesystem.write(destination, &bytes), destination.as_str())
}

#[derive(Debug)]
enum CopyEntry {
    Directory { destination: VPath, mode: u32 },
    File { destination: VPath, bytes: Vec<u8> },
}

fn copy_tree(
    source: &VPath,
    destination: &VPath,
    source_state: NodeState,
    overwrite: bool,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<(), CallFailure> {
    match filesystem.metadata(destination) {
        Ok(_) if !overwrite => return Err(already_exists(destination.as_str())),
        Ok(_) => {
            return Err(value_error(
                "vsh_copy() cannot merge or overwrite a directory tree".to_owned(),
            ));
        }
        Err(VfsError::NotFound { .. }) => {}
        Err(error) => return Err(classify_vfs(error, destination.as_str())),
    }
    let mut entries = vec![CopyEntry::Directory {
        destination: destination.clone(),
        mode: source_state.mode(),
    }];
    let mut pending = vec![source.clone()];
    while let Some(directory) = pending.pop() {
        authorize_path(config, &directory, &[AccessKind::DirectoryRead])?;
        let children = vfs(filesystem.read_dir(&directory), directory.as_str())?;
        budget.charge_directory_entries(children.len())?;
        for child in children {
            authorize_path(config, &child, &[AccessKind::MetadataRead])?;
            let state = vfs(filesystem.metadata(&child), child.as_str())?;
            let target = child
                .rebase(source, destination)
                .map_err(|error| CallFailure::InternalVfs(VfsError::Path(error)))?
                .expect("walked copy child must be within source");
            authorize_path(config, &target, &[AccessKind::Create, AccessKind::Modify])?;
            match state.kind() {
                NodeKind::Directory => {
                    entries.push(CopyEntry::Directory {
                        destination: target,
                        mode: state.mode(),
                    });
                    pending.push(child);
                }
                NodeKind::File => {
                    authorize_path(config, &child, &[AccessKind::ContentRead])?;
                    budget.charge_read(state.size())?;
                    let bytes = vfs(filesystem.read(&child), child.as_str())?;
                    budget.charge_write(bytes.len())?;
                    entries.push(CopyEntry::File {
                        destination: target,
                        bytes,
                    });
                }
                NodeKind::Symlink => {
                    return Err(value_error(format!(
                        "vsh_copy() does not copy symbolic link {:?}",
                        child.as_str()
                    )));
                }
            }
        }
    }
    entries.sort_by_key(|entry| match entry {
        CopyEntry::Directory { destination, .. } | CopyEntry::File { destination, .. } => {
            destination.as_str().matches('/').count()
        }
    });
    for entry in entries {
        match entry {
            CopyEntry::Directory { destination, mode } => {
                vfs(filesystem.mkdir(&destination, mode), destination.as_str())?;
            }
            CopyEntry::File { destination, bytes } => {
                vfs(filesystem.write(&destination, &bytes), destination.as_str())?;
            }
        }
    }
    Ok(())
}

fn glob(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind(
        "vsh_glob",
        args,
        kwargs,
        &["pattern", "path", "max_results"],
    )?;
    let pattern = args.required_string("pattern")?;
    validate_pattern(pattern, config)?;
    let raw_root = args.optional_path("path", config.virtual_root.as_str())?;
    let max_results = args.optional_usize("max_results", 1_000)?;
    let marker = OsFunctionCall::Iterdir(MontyPath::from(raw_root));
    let root = map_authorized_path(
        &marker,
        raw_root,
        false,
        config,
        &[AccessKind::MetadataRead],
    )?;
    if max_results == 0 {
        let state = vfs(filesystem.metadata(&root), raw_root)?;
        if state.kind() != NodeKind::Directory {
            return Err(not_directory(raw_root));
        }
        authorize_path(config, &root, &[AccessKind::DirectoryRead])?;
        return Ok(MontyObject::List(Vec::new()));
    }
    let normalized = pattern.trim_start_matches("./").replace('\\', "/");
    let compiled = GlobPattern::new(&normalized);
    let mut paths = Vec::with_capacity(max_results.min(64));
    walk_visible(&root, filesystem, config, budget, |path, _, _, _| {
        if path
            .relative_to(&root)
            .is_some_and(|relative| compiled.matches(relative))
        {
            paths.push(MontyObject::Path(config.virtual_root.present(path)));
        }
        Ok(paths.len() < max_results)
    })?;
    Ok(MontyObject::List(paths))
}

fn search(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind(
        "vsh_search",
        args,
        kwargs,
        &["query", "path", "case_sensitive", "max_results"],
    )?;
    let query = args.required_string("query")?;
    if query.is_empty() {
        return Err(value_error("vsh_search() query cannot be empty".to_owned()));
    }
    let raw_root = args.optional_path("path", config.virtual_root.as_str())?;
    let case_sensitive = args.optional_bool("case_sensitive", true)?;
    let max_results = args.optional_usize("max_results", 100)?;
    let marker = OsFunctionCall::Iterdir(MontyPath::from(raw_root));
    let root = map_authorized_path(
        &marker,
        raw_root,
        false,
        config,
        &[AccessKind::MetadataRead],
    )?;
    let root_state = vfs(filesystem.metadata(&root), raw_root)?;
    if max_results == 0 {
        if root_state.kind() == NodeKind::Directory {
            authorize_path(config, &root, &[AccessKind::DirectoryRead])?;
        }
        return Ok(MontyObject::List(Vec::new()));
    }
    let folded_query = (!case_sensitive).then(|| query.to_lowercase());
    let mut matches = Vec::with_capacity(max_results.min(64));
    if root_state.kind() == NodeKind::Directory {
        walk_visible(
            &root,
            filesystem,
            config,
            budget,
            |path, state, filesystem, budget| {
                search_file(
                    path,
                    state,
                    query,
                    folded_query.as_deref(),
                    max_results,
                    &mut matches,
                    filesystem,
                    config,
                    budget,
                )
            },
        )?;
    } else {
        authorize_path(config, &root, &[AccessKind::ContentRead])?;
        search_file(
            &root,
            root_state,
            query,
            folded_query.as_deref(),
            max_results,
            &mut matches,
            filesystem,
            config,
            budget,
        )?;
    }
    Ok(MontyObject::List(matches))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the bounded visitor keeps search on one pass without a result-tree allocation"
)]
fn search_file(
    path: &VPath,
    state: NodeState,
    query: &str,
    folded_query: Option<&str>,
    max_results: usize,
    matches: &mut Vec<MontyObject>,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<bool, CallFailure> {
    if state.kind() != NodeKind::File
        || config
            .call_policy
            .authorize(path, AccessKind::ContentRead)
            .is_err()
    {
        return Ok(true);
    }
    budget.charge_read(state.size())?;
    let bytes = vfs(filesystem.read(path), path.as_str())?;
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(true);
    };
    for (line_index, line) in text.lines().enumerate() {
        let Some(column) = search_column(line, query, folded_query) else {
            continue;
        };
        matches.push(search_match(
            config.virtual_root.present(path),
            line_index.saturating_add(1),
            column,
            line,
        ));
        if matches.len() >= max_results {
            return Ok(false);
        }
    }
    Ok(true)
}

fn search_column(line: &str, query: &str, folded_query: Option<&str>) -> Option<usize> {
    let Some(folded_query) = folded_query else {
        let byte_index = line.find(query)?;
        return Some(line[..byte_index].chars().count().saturating_add(1));
    };

    let folded_line = line.to_lowercase();
    let folded_byte_index = folded_line.find(folded_query)?;
    let mut folded_offset = 0_usize;
    for (column, character) in line.chars().enumerate() {
        let next_offset = folded_offset
            .saturating_add(character.to_lowercase().map(char::len_utf8).sum::<usize>());
        if folded_byte_index < next_offset {
            return Some(column.saturating_add(1));
        }
        folded_offset = next_offset;
    }
    None
}

fn search_match(path: String, line: usize, column: usize, text: &str) -> MontyObject {
    MontyObject::Dict(DictPairs::from(vec![
        (
            MontyObject::String("path".to_owned()),
            MontyObject::Path(path),
        ),
        (
            MontyObject::String("line".to_owned()),
            MontyObject::Int(i64::try_from(line).unwrap_or(i64::MAX)),
        ),
        (
            MontyObject::String("column".to_owned()),
            MontyObject::Int(i64::try_from(column).unwrap_or(i64::MAX)),
        ),
        (
            MontyObject::String("text".to_owned()),
            MontyObject::String(text.to_owned()),
        ),
    ]))
}

fn patch(
    args: &[MontyObject],
    kwargs: &[(MontyObject, MontyObject)],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let args = Arguments::bind("vsh_patch", args, kwargs, &["path", "old", "new", "count"])?;
    let path = args.required_path("path")?;
    let old = args.required_string("old")?;
    let new = args.required_string("new")?;
    let count = args.optional_usize("count", 1)?;
    if old.is_empty() {
        return Err(value_error(
            "vsh_patch() old text cannot be empty".to_owned(),
        ));
    }
    if count == 0 {
        return Err(value_error("vsh_patch() count must be positive".to_owned()));
    }
    let read_call = OsFunctionCall::ReadText(MontyPath::from(path));
    let MontyObject::String(contents) = dispatch_call(&read_call, filesystem, config, budget)?
    else {
        unreachable!("ReadText always returns a string");
    };
    let replacements = contents.match_indices(old).take(count).count();
    if replacements == 0 {
        return Err(value_error(format!(
            "vsh_patch() could not find the requested text in {path:?}"
        )));
    }
    let updated = contents.replacen(old, new, count);
    let write_call = OsFunctionCall::WriteText(PathStringDataArgs {
        path: MontyPath::from(path),
        data: updated,
    });
    dispatch_call(&write_call, filesystem, config, budget)?;
    Ok(MontyObject::Int(
        i64::try_from(replacements).unwrap_or(i64::MAX),
    ))
}

fn walk_visible(
    root: &VPath,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
    mut visit: impl FnMut(&VPath, NodeState, &mut VirtualFs, &mut Budget) -> Result<bool, CallFailure>,
) -> Result<(), CallFailure> {
    let state = vfs(filesystem.metadata(root), root.as_str())?;
    if state.kind() != NodeKind::Directory {
        return Err(not_directory(root.as_str()));
    }
    authorize_path(config, root, &[AccessKind::DirectoryRead])?;
    let children = vfs(filesystem.read_dir(root), root.as_str())?;
    budget.charge_directory_entries(children.len())?;
    let mut pending = children.into_iter().rev().collect::<Vec<_>>();
    while let Some(path) = pending.pop() {
        if config
            .call_policy
            .authorize(&path, AccessKind::MetadataRead)
            .is_err()
        {
            continue;
        }
        let state = vfs(filesystem.metadata(&path), path.as_str())?;
        if !visit(&path, state, filesystem, budget)? {
            return Ok(());
        }
        if state.kind() == NodeKind::Directory
            && config
                .call_policy
                .authorize(&path, AccessKind::DirectoryRead)
                .is_ok()
        {
            let children = vfs(filesystem.read_dir(&path), path.as_str())?;
            budget.charge_directory_entries(children.len())?;
            pending.extend(children.into_iter().rev());
        }
    }
    Ok(())
}

fn validate_pattern(pattern: &str, config: &InProcessConfig) -> Result<(), CallFailure> {
    if pattern.is_empty() {
        return Err(value_error("vsh_glob() pattern cannot be empty".to_owned()));
    }
    check_path_bytes(pattern, config)?;
    if pattern.contains('\0')
        || pattern.starts_with('/')
        || pattern
            .replace('\\', "/")
            .split('/')
            .any(|component| component == "..")
    {
        return Err(value_error(
            "vsh_glob() pattern must be relative and remain inside its root".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
enum GlobComponent {
    Recursive,
    Pattern(Vec<char>),
}

#[derive(Debug)]
struct GlobPattern {
    components: Vec<GlobComponent>,
}

impl GlobPattern {
    fn new(pattern: &str) -> Self {
        let components = pattern
            .split('/')
            .map(|component| {
                if component == "**" {
                    GlobComponent::Recursive
                } else {
                    GlobComponent::Pattern(component.chars().collect())
                }
            })
            .collect();
        Self { components }
    }

    fn matches(&self, path: &str) -> bool {
        let path = path
            .split('/')
            .map(|component| component.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut pattern_index = 0;
        let mut path_index = 0;
        let mut recursive_index = None;
        let mut recursive_path_index = 0;

        while path_index < path.len() {
            match self.components.get(pattern_index) {
                Some(GlobComponent::Recursive) => {
                    recursive_index = Some(pattern_index);
                    recursive_path_index = path_index;
                    pattern_index = pattern_index.saturating_add(1);
                }
                Some(GlobComponent::Pattern(pattern))
                    if component_matches_chars(pattern, &path[path_index]) =>
                {
                    pattern_index = pattern_index.saturating_add(1);
                    path_index = path_index.saturating_add(1);
                }
                _ => {
                    let Some(recursive_index) = recursive_index else {
                        return false;
                    };
                    recursive_path_index = recursive_path_index.saturating_add(1);
                    path_index = recursive_path_index;
                    pattern_index = recursive_index.saturating_add(1);
                }
            }
        }
        while matches!(
            self.components.get(pattern_index),
            Some(GlobComponent::Recursive)
        ) {
            pattern_index = pattern_index.saturating_add(1);
        }
        pattern_index == self.components.len()
    }
}

#[cfg(test)]
fn glob_matches(pattern: &str, path: &str) -> bool {
    GlobPattern::new(pattern).matches(path)
}

#[cfg(test)]
fn component_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    component_matches_chars(&pattern, &value)
}

fn component_matches_chars(pattern: &[char], value: &[char]) -> bool {
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star_index = None;
    let mut star_value_index = 0;

    while value_index < value.len() {
        match pattern.get(pattern_index) {
            Some('?') => {
                pattern_index = pattern_index.saturating_add(1);
                value_index = value_index.saturating_add(1);
            }
            Some('*') => {
                star_index = Some(pattern_index);
                star_value_index = value_index;
                pattern_index = pattern_index.saturating_add(1);
            }
            Some(literal) if *literal == value[value_index] => {
                pattern_index = pattern_index.saturating_add(1);
                value_index = value_index.saturating_add(1);
            }
            _ => {
                let Some(star_index) = star_index else {
                    return false;
                };
                star_value_index = star_value_index.saturating_add(1);
                value_index = star_value_index;
                pattern_index = star_index.saturating_add(1);
            }
        }
    }
    while matches!(pattern.get(pattern_index), Some('*')) {
        pattern_index = pattern_index.saturating_add(1);
    }
    pattern_index == pattern.len()
}

struct Arguments<'a> {
    function: &'static str,
    positional: &'a [MontyObject],
    keywords: BTreeMap<&'a str, &'a MontyObject>,
    parameters: &'static [&'static str],
}

impl<'a> Arguments<'a> {
    fn bind(
        function: &'static str,
        positional: &'a [MontyObject],
        keywords: &'a [(MontyObject, MontyObject)],
        parameters: &'static [&'static str],
    ) -> Result<Self, CallFailure> {
        if positional.len() > parameters.len() {
            return Err(type_error(format!(
                "{function}() takes at most {} arguments ({} given)",
                parameters.len(),
                positional.len()
            )));
        }
        let mut bound_keywords = BTreeMap::new();
        for (key, value) in keywords {
            let MontyObject::String(key) = key else {
                return Err(type_error(format!("{function}() keywords must be strings")));
            };
            let Some(index) = parameters.iter().position(|parameter| *parameter == key) else {
                return Err(type_error(format!(
                    "{function}() got an unexpected keyword argument {key:?}"
                )));
            };
            if index < positional.len() || bound_keywords.insert(key.as_str(), value).is_some() {
                return Err(type_error(format!(
                    "{function}() got multiple values for argument {key:?}"
                )));
            }
        }
        Ok(Self {
            function,
            positional,
            keywords: bound_keywords,
            parameters,
        })
    }

    fn get(&self, name: &str) -> Option<&'a MontyObject> {
        let index = self
            .parameters
            .iter()
            .position(|parameter| *parameter == name)
            .expect("argument helper must request a declared parameter");
        self.positional
            .get(index)
            .or_else(|| self.keywords.get(name).copied())
    }

    fn required(&self, name: &str) -> Result<&'a MontyObject, CallFailure> {
        self.get(name).ok_or_else(|| {
            type_error(format!(
                "{}() missing required argument {name:?}",
                self.function
            ))
        })
    }

    fn required_path(&self, name: &str) -> Result<&'a str, CallFailure> {
        self.path_value(name, self.required(name)?)
    }

    fn optional_path(&self, name: &str, default: &'a str) -> Result<&'a str, CallFailure> {
        self.get(name)
            .map_or(Ok(default), |value| self.path_value(name, value))
    }

    fn path_value(&self, name: &str, value: &'a MontyObject) -> Result<&'a str, CallFailure> {
        match value {
            MontyObject::String(path) | MontyObject::Path(path) => Ok(path),
            value => Err(type_error(format!(
                "{}() argument {name:?} must be str or Path, not {}",
                self.function,
                value.type_name()
            ))),
        }
    }

    fn required_string(&self, name: &str) -> Result<&'a str, CallFailure> {
        match self.required(name)? {
            MontyObject::String(value) => Ok(value),
            value => Err(type_error(format!(
                "{}() argument {name:?} must be str, not {}",
                self.function,
                value.type_name()
            ))),
        }
    }

    fn optional_bool(&self, name: &str, default: bool) -> Result<bool, CallFailure> {
        match self.get(name) {
            None => Ok(default),
            Some(MontyObject::Bool(value)) => Ok(*value),
            Some(value) => Err(type_error(format!(
                "{}() argument {name:?} must be bool, not {}",
                self.function,
                value.type_name()
            ))),
        }
    }

    fn optional_usize(&self, name: &str, default: usize) -> Result<usize, CallFailure> {
        match self.get(name) {
            None => Ok(default),
            Some(MontyObject::Int(value)) => usize::try_from(*value).map_err(|_| {
                value_error(format!(
                    "{}() argument {name:?} must be a non-negative integer",
                    self.function
                ))
            }),
            Some(value) => Err(type_error(format!(
                "{}() argument {name:?} must be int, not {}",
                self.function,
                value.type_name()
            ))),
        }
    }
}

fn type_error(message: String) -> CallFailure {
    CallFailure::Python(MontyException::new(ExcType::TypeError, Some(message)))
}

fn value_error(message: String) -> CallFailure {
    CallFailure::Python(MontyException::new(ExcType::ValueError, Some(message)))
}

fn runtime_error(message: String) -> CallFailure {
    CallFailure::Python(MontyException::new(ExcType::RuntimeError, Some(message)))
}

#[cfg(test)]
mod tests {
    use super::{component_matches, glob_matches, search_column};

    #[test]
    fn component_glob_supports_star_and_question_mark() {
        assert!(component_matches("*.rs", "lib.rs"));
        assert!(component_matches("t?st.rs", "test.rs"));
        assert!(component_matches("*a*b", "zzaxxb"));
        assert!(component_matches("?.txt", "ö.txt"));
        assert!(!component_matches("*.py", "lib.rs"));
        assert!(!component_matches("*a*b", "zzaxxc"));
    }

    #[test]
    fn recursive_glob_matches_zero_or_more_components() {
        assert!(glob_matches("**/*.rs", "lib.rs"));
        assert!(glob_matches("**/*.rs", "src/deep/lib.rs"));
        assert!(glob_matches("**/src/**/lib.rs", "prefix/src/deep/lib.rs"));
        assert!(!glob_matches("src/*.rs", "src/deep/lib.rs"));
    }

    #[test]
    fn recursive_glob_depth_is_iterative() {
        let mut components = vec!["**"; 2_000];
        components.push("target.txt");
        assert!(glob_matches(&components.join("/"), "target.txt"));
    }

    #[test]
    fn case_insensitive_search_maps_expanding_unicode_to_original_column() {
        assert_eq!(search_column("xİz", "i", Some("i")), Some(2));
        assert_eq!(search_column("öNE", "öne", Some("öne")), Some(1));
    }
}
