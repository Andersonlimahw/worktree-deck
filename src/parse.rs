use crate::error::AppError;
use crate::model::{ChangeCounts, Worktree};

/// Parse `git worktree list --porcelain -z`.
///
/// Records are separated by a blank line, which `-z` encodes as two NULs.
/// The first record is the main worktree.
pub(crate) fn parse_worktree_list(data: &[u8]) -> Result<Vec<Worktree>, AppError> {
    let mut worktrees = Vec::new();
    for record in split_records(data) {
        if record.is_empty() || record.iter().all(|byte| *byte == 0) {
            continue;
        }
        worktrees.push(parse_worktree_record(record)?);
    }
    if let Some(first) = worktrees.first_mut() {
        first.is_main = true;
    }
    Ok(worktrees)
}

/// Parse `git status --porcelain=v2 -z`.
pub(crate) fn parse_status_v2(data: &[u8]) -> Result<ChangeCounts, AppError> {
    let mut counts = ChangeCounts {
        staged: 0,
        unstaged: 0,
        untracked: 0,
    };
    let mut index = 0;
    while index < data.len() {
        if data[index] == 0 {
            index += 1;
            continue;
        }
        let end = find_nul(data, index);
        let line = utf8(&data[index..end])?;
        let rename = line.starts_with("2 ");
        index = advance_past_nul(data, end);
        if rename {
            index = consume_rename_source(data, index)?;
        }
        if line.is_empty() {
            continue;
        }
        apply_status_line(&mut counts, line)?;
    }
    Ok(counts)
}

fn parse_worktree_record(record: &[u8]) -> Result<Worktree, AppError> {
    let mut path = None;
    let mut head = None;
    let mut ref_name = None;
    let mut detached = false;
    let mut bare = false;
    let mut locked = false;
    let mut lock_reason = None;
    let mut prunable = false;
    let mut prunable_reason = None;

    for raw_line in record.split(|byte| *byte == 0) {
        if raw_line.is_empty() {
            continue;
        }
        let line = utf8(raw_line)?;
        let (key, rest) = split_key(line);
        match key {
            "worktree" => {
                path = Some(
                    rest.ok_or_else(|| AppError::invalid_git("worktree entry is missing a path"))?
                        .to_string(),
                );
            }
            "HEAD" => {
                head = Some(
                    rest.ok_or_else(|| AppError::invalid_git("HEAD entry is missing an oid"))?
                        .to_string(),
                );
            }
            "branch" => {
                ref_name = Some(
                    rest.ok_or_else(|| AppError::invalid_git("branch entry is missing a ref"))?
                        .to_string(),
                );
            }
            "detached" => detached = true,
            "bare" => bare = true,
            "locked" => {
                locked = true;
                lock_reason = optional_text(rest);
            }
            "prunable" => {
                prunable = true;
                prunable_reason = optional_text(rest);
            }
            _ => {
                // Unknown attributes are ignored so a newer git can add fields
                // without breaking listing. Status parsing fails closed instead.
            }
        }
    }

    let path = path.ok_or_else(|| AppError::invalid_git("worktree record has no path"))?;
    if !bare && !detached && ref_name.is_none() {
        return Err(AppError::invalid_git(format!(
            "worktree {path} has no branch, detached, or bare attribute"
        )));
    }
    if detached {
        ref_name = None;
    }
    let branch = ref_name.as_deref().map(short_branch);
    Ok(Worktree {
        path,
        head: head.unwrap_or_default(),
        branch,
        ref_name,
        detached,
        bare,
        is_main: false,
        locked,
        lock_reason,
        prunable,
        prunable_reason,
        changes: None,
    })
}

fn apply_status_line(counts: &mut ChangeCounts, line: &str) -> Result<(), AppError> {
    if line.starts_with('#') {
        return Ok(());
    }
    if line.starts_with("? ") {
        counts.untracked += 1;
        return Ok(());
    }
    if line.starts_with("! ") {
        return Ok(());
    }
    if line.starts_with("1 ") || line.starts_with("2 ") || line.starts_with("u ") {
        let xy = line
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| AppError::invalid_git("status line is missing the XY field"))?;
        if xy.chars().count() != 2 {
            return Err(AppError::invalid_git(format!(
                "status XY field is not two characters: {xy}"
            )));
        }
        let mut chars = xy.chars();
        let index_status = chars.next().unwrap_or('.');
        let worktree_status = chars.next().unwrap_or('.');
        // Anything other than '.' counts. Over-counting refuses a removal,
        // which is the safe direction.
        if index_status != '.' {
            counts.staged += 1;
        }
        if worktree_status != '.' {
            counts.unstaged += 1;
        }
        return Ok(());
    }
    Err(AppError::invalid_git(format!(
        "unrecognized status line: {line}"
    )))
}

fn consume_rename_source(data: &[u8], index: usize) -> Result<usize, AppError> {
    if index >= data.len() {
        return Err(AppError::invalid_git(
            "rename status entry is missing the original path",
        ));
    }
    let end = find_nul(data, index);
    if end >= data.len() || data[end] != 0 {
        return Err(AppError::invalid_git(
            "rename status entry is missing the original path",
        ));
    }
    utf8(&data[index..end])?;
    Ok(end + 1)
}

fn split_records(data: &[u8]) -> Vec<&[u8]> {
    let mut records = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < data.len() {
        if data[index] == 0 && index + 1 < data.len() && data[index + 1] == 0 {
            if index > start {
                records.push(&data[start..index]);
            }
            index += 2;
            start = index;
        } else {
            index += 1;
        }
    }
    if start < data.len() {
        let mut end = data.len();
        if data[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            records.push(&data[start..end]);
        }
    }
    records
}

fn find_nul(data: &[u8], start: usize) -> usize {
    data[start..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|offset| start + offset)
        .unwrap_or(data.len())
}

fn advance_past_nul(data: &[u8], end: usize) -> usize {
    if end < data.len() {
        end + 1
    } else {
        end
    }
}

fn split_key(line: &str) -> (&str, Option<&str>) {
    match line.split_once(' ') {
        Some((key, rest)) => (key, Some(rest)),
        None => (line, None),
    }
}

fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn short_branch(ref_name: &str) -> String {
    ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(ref_name)
        .to_string()
}

fn utf8(bytes: &[u8]) -> Result<&str, AppError> {
    std::str::from_utf8(bytes).map_err(|_| AppError::invalid_git("git output is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(text: &str) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    #[test]
    fn parses_real_worktree_shapes() {
        let data = record(concat!(
            "worktree /tmp/repo\0HEAD abcdef\0branch refs/heads/main\0\0",
            "worktree /tmp/my worktree\0HEAD abcdef\0branch refs/heads/feature/sub\0\0",
            "worktree /tmp/árvore\0HEAD abcdef\0detached\0\0",
            "worktree /tmp/locked\0HEAD abcdef\0branch refs/heads/locked-b\0locked do not touch\0\0",
            "worktree /tmp/lock2\0HEAD abcdef\0branch refs/heads/lock2\0locked\0\0",
            "worktree /tmp/gone\0HEAD abcdef\0branch refs/heads/gone\0prunable gitdir file points to non-existent location\0\0",
        ));
        let worktrees = parse_worktree_list(&data).unwrap();
        assert_eq!(worktrees.len(), 6);
        assert!(worktrees[0].is_main);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(worktrees[0].ref_name.as_deref(), Some("refs/heads/main"));
        assert!(!worktrees[1].is_main);
        assert_eq!(worktrees[1].path, "/tmp/my worktree");
        assert_eq!(worktrees[1].branch.as_deref(), Some("feature/sub"));
        assert_eq!(worktrees[2].path, "/tmp/árvore");
        assert!(worktrees[2].detached);
        assert!(worktrees[2].branch.is_none());
        assert!(worktrees[3].locked);
        assert_eq!(worktrees[3].lock_reason.as_deref(), Some("do not touch"));
        assert!(worktrees[4].locked);
        assert!(worktrees[4].lock_reason.is_none());
        assert!(worktrees[5].prunable);
        assert!(worktrees[5]
            .prunable_reason
            .as_deref()
            .unwrap()
            .contains("non-existent"));
    }

    #[test]
    fn parses_status_counts_and_skips_ignored_headers() {
        let data = record(concat!(
            "# branch.oid abcdef\0",
            "1 MM N... 100644 100644 100644 aaa bbb file with space.txt\0",
            "1 M. N... 100644 100644 100644 aaa bbb staged.txt\0",
            "1 .M N... 100644 100644 100644 aaa bbb unstaged.txt\0",
            "? nested/new file.txt\0",
            "! ignored.txt\0",
            "2 R. N... 100644 100644 100644 aaa bbb R100 new name\0old name\0",
            "u UU N... 100644 100644 100644 100644 a b c conflict.txt\0",
        ));
        let counts = parse_status_v2(&data).unwrap();
        assert_eq!(counts.staged, 4);
        assert_eq!(counts.unstaged, 3);
        assert_eq!(counts.untracked, 1);
    }

    #[test]
    fn empty_status_is_clean() {
        let counts = parse_status_v2(b"").unwrap();
        assert!(!counts.is_dirty());
    }

    #[test]
    fn unknown_status_line_fails_closed() {
        let error = parse_status_v2(b"3 XX mystery\0").unwrap_err();
        assert!(error.message.contains("unrecognized"));
    }

    #[test]
    fn rename_without_source_fails() {
        let error = parse_status_v2(b"2 R. N... 100644 100644 100644 a b R100 only\0").unwrap_err();
        assert!(error.message.contains("original path"));
    }
}
