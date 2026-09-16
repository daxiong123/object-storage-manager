//! 对象列表排序与「排序 × 过滤」的显示顺序。

use super::*;

/// 对象列表排序方式（表头点击切换；Finder 式：目录恒在对象前）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ObjectSort {
    /// 列举原序（provider 返回顺序；字典序）
    #[default]
    Natural,
    /// 名称（A→Z，大小写不敏感）
    NameAsc,
    /// 名称（Z→A）
    NameDesc,
    /// 大小（大→小）
    SizeDesc,
    /// 修改时间（新→旧）
    TimeDesc,
}

impl ObjectSort {
    pub(super) fn cycle_name(self) -> Self {
        match self {
            ObjectSort::NameAsc => ObjectSort::NameDesc,
            ObjectSort::NameDesc => ObjectSort::Natural,
            _ => ObjectSort::NameAsc,
        }
    }

    pub(super) fn cycle_size(self) -> Self {
        match self {
            ObjectSort::SizeDesc => ObjectSort::Natural,
            _ => ObjectSort::SizeDesc,
        }
    }

    pub(super) fn cycle_time(self) -> Self {
        match self {
            ObjectSort::TimeDesc => ObjectSort::Natural,
            _ => ObjectSort::TimeDesc,
        }
    }

    pub(super) fn name_mark(self) -> &'static str {
        match self {
            ObjectSort::NameAsc => " ↑",
            ObjectSort::NameDesc => " ↓",
            _ => "",
        }
    }

    pub(super) fn size_mark(self) -> &'static str {
        match self {
            ObjectSort::SizeDesc => " ↓",
            _ => "",
        }
    }

    pub(super) fn time_mark(self) -> &'static str {
        match self {
            ObjectSort::TimeDesc => " ↓",
            _ => "",
        }
    }

    pub(super) fn label(self) -> Option<&'static str> {
        match self {
            ObjectSort::Natural => None,
            ObjectSort::NameAsc => Some("名称 A→Z"),
            ObjectSort::NameDesc => Some("名称 Z→A"),
            ObjectSort::SizeDesc => Some("最大优先"),
            ObjectSort::TimeDesc => Some("最新优先"),
        }
    }
}

/// 排序纯函数：输入 entries 与排序方式，返回展示顺序下标（指向 entries）。
/// 不改 entries 本身（选择锚点/过滤命中缓存都以原下标为基准）。
/// 目录恒在对象前（Finder 语义），对象内部按所选键排序。
pub(crate) fn sort_entries(entries: &[ListingEntry], sort: ObjectSort) -> Vec<usize> {
    let mut ix: Vec<usize> = (0..entries.len()).collect();
    if matches!(sort, ObjectSort::Natural) {
        return ix;
    }
    // 排序键：(段类型序 目录=0 对象=1, 降序数值键, 字符串键)。
    // 数值键包 Reverse 实现大→小/新→旧；NameAsc/Desc 数值键恒 0 不参与。
    let key = |entry: &ListingEntry| -> (usize, std::cmp::Reverse<u64>, String) {
        match entry {
            ListingEntry::CommonPrefix(p) => (0, std::cmp::Reverse(0), p.to_lowercase()),
            ListingEntry::Object(o) => match sort {
                ObjectSort::SizeDesc => (1, std::cmp::Reverse(o.size), o.key.to_lowercase()),
                ObjectSort::TimeDesc => (
                    1,
                    std::cmp::Reverse(o.put_time_millis.max(0) as u64),
                    o.key.to_lowercase(),
                ),
                _ => (1, std::cmp::Reverse(0), o.key.to_lowercase()),
            },
        }
    };
    match sort {
        ObjectSort::NameAsc => ix.sort_by(|&a, &b| {
            let (ta, _, na) = key(&entries[a]);
            let (tb, _, nb) = key(&entries[b]);
            (ta, na).cmp(&(tb, nb))
        }),
        ObjectSort::NameDesc => ix.sort_by(|&a, &b| {
            let (ta, _, na) = key(&entries[a]);
            let (tb, _, nb) = key(&entries[b]);
            (ta, std::cmp::Reverse(na)).cmp(&(tb, std::cmp::Reverse(nb)))
        }),
        _ => ix.sort_by_key(|&i| key(&entries[i])),
    }
    ix
}

/// 展示顺序：先按排序取全量下标，再与过滤命中求交（过滤不影响 entries 本身）。
pub(crate) fn display_entry_order(
    entries: &[ListingEntry],
    sort: ObjectSort,
    filtered_ix: Option<&[usize]>,
) -> Vec<usize> {
    let sorted = sort_entries(entries, sort);
    match filtered_ix {
        Some(ix) => {
            let keep: std::collections::HashSet<usize> = ix.iter().copied().collect();
            sorted.into_iter().filter(|i| keep.contains(i)).collect()
        }
        None => sorted,
    }
}
