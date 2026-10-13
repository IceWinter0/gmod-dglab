//! 只读解析当前构建的客户端接收属性元数据，不调用游戏虚函数。
use crate::native;
use std::{collections::HashSet, io};

#[derive(Debug, PartialEq, Eq)]
struct Property {
    name: usize,
    kind: i32,
    child: usize,
    offset: usize,
    inside_array: bool,
}
fn usize_at(bytes: &[u8], offset: usize) -> Option<usize> {
    Some(usize::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}
fn i32_at(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn parse_property(bytes: &[u8]) -> Option<Property> {
    if bytes.len() < 96 {
        return None;
    }
    let offset = usize::try_from(i32_at(bytes, 72)?).ok()?;
    let kind = i32_at(bytes, 8)?;
    if offset > 0x100000 || !(0..=7).contains(&kind) {
        return None;
    }
    Some(Property {
        name: usize_at(bytes, 0)?,
        kind,
        child: usize_at(bytes, 64)?,
        offset,
        inside_array: bytes[20] != 0,
    })
}

fn pointer(address: usize) -> io::Result<usize> {
    native::read::<8>(address)
        .map(usize::from_le_bytes)
        .ok_or_else(|| io::Error::other("属性元数据指针不可读"))
}
fn text(address: usize) -> io::Result<String> {
    if let Some(bytes) = native::read::<128>(address) {
        if let Some(end) = bytes.iter().position(|b| *b == 0) {
            return std::str::from_utf8(&bytes[..end])
                .map(str::to_owned)
                .map_err(io::Error::other);
        }
    }
    let mut bytes = Vec::new();
    for i in 0..128 {
        let b = native::read::<1>(
            address
                .checked_add(i)
                .ok_or_else(|| io::Error::other("属性名称地址溢出"))?,
        )
        .ok_or_else(|| io::Error::other("属性名称不可读"))?[0];
        if b == 0 {
            return String::from_utf8(bytes).map_err(io::Error::other);
        }
        bytes.push(b);
    }
    Err(io::Error::other("属性名称过长"))
}

fn walk(
    table: usize,
    base: usize,
    path: &str,
    parents: &mut HashSet<usize>,
    lines: &mut Vec<String>,
) -> io::Result<()> {
    if parents.len() >= 32 || !parents.insert(table) {
        return Err(io::Error::other("属性表递归过深或存在环"));
    }
    let data = native::read::<40>(table).ok_or_else(|| io::Error::other("属性表不可读"))?;
    let props = usize_at(&data, 0).ok_or_else(|| io::Error::other("属性表指针无效"))?;
    let count = i32_at(&data, 8).ok_or_else(|| io::Error::other("属性数量不可读"))?;
    if !(0..=4096).contains(&count) {
        return Err(io::Error::other("属性数量超出范围"));
    }
    for index in 0..count as usize {
        let address = props
            .checked_add(index * 96)
            .ok_or_else(|| io::Error::other("属性地址溢出"))?;
        let data = native::read::<96>(address).ok_or_else(|| io::Error::other("属性记录不可读"))?;
        let Some(prop) = parse_property(&data) else {
            continue;
        };
        if prop.inside_array {
            continue;
        }
        let name = text(prop.name)?;
        let offset = base
            .checked_add(prop.offset)
            .filter(|n| *n <= 0x100000)
            .ok_or_else(|| io::Error::other("嵌套属性偏移无效"))?;
        let child_path = format!("{path}/{name}");
        if matches!(
            name.as_str(),
            "m_iHealth" | "m_iArmor" | "m_ArmorValue" | "m_lifeState" | "m_iMaxHealth"
        ) {
            lines.push(format!(
                "  {child_path}：偏移 0x{offset:x}，类型 {}，代理 RVA 另待核对",
                prop.kind
            ));
        }
        if prop.kind == 6 && prop.child != 0 {
            walk(prop.child, offset, &child_path, parents, lines)?;
        }
    }
    parents.remove(&table);
    Ok(())
}

pub fn inspect(client_base: usize) -> io::Result<String> {
    // 0x95F748 来自当前已验证构建 GetAllClasses 的 RIP 相对加载，避免工作线程调用游戏方法。
    let mut class = pointer(client_base + 0x95f748)?;
    let mut visited = HashSet::new();
    let mut lines = vec![];
    while class != 0 {
        if visited.len() >= 2048 || !visited.insert(class) {
            return Err(io::Error::other("客户端类链表超出边界或存在环"));
        }
        let data =
            native::read::<48>(class).ok_or_else(|| io::Error::other("客户端类记录不可读"))?;
        let name = text(usize_at(&data, 16).ok_or_else(|| io::Error::other("客户端类名称无效"))?)?;
        let table = usize_at(&data, 24).ok_or_else(|| io::Error::other("客户端类属性表无效"))?;
        if name.to_ascii_lowercase().contains("player") && table != 0 {
            let metadata =
                native::read::<40>(table).ok_or_else(|| io::Error::other("玩家属性表不可读"))?;
            let table_name = text(
                usize_at(&metadata, 24).ok_or_else(|| io::Error::other("玩家属性表名称无效"))?,
            )?;
            lines.push(format!("玩家候选类：{name} / {table_name}"));
            walk(table, 0, &table_name, &mut HashSet::new(), &mut lines)?;
        }
        class = usize_at(&data, 32).ok_or_else(|| io::Error::other("客户端类后继无效"))?;
    }
    if visited.is_empty() || lines.is_empty() {
        return Err(io::Error::other("尚未发现玩家客户端类，属性解析未完成"));
    }
    Ok(format!(
        "已遍历 {} 个客户端类；以下是玩家属性候选，不视为已完成采样：\n{}",
        visited.len(),
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_bounded_x64_receive_property_metadata() {
        let mut bytes = [0u8; 96];
        bytes[..8].copy_from_slice(&0x18068b8c8u64.to_le_bytes());
        bytes[64..72].copy_from_slice(&0x18093b000u64.to_le_bytes());
        bytes[72..76].copy_from_slice(&0xd8i32.to_le_bytes());
        assert_eq!(
            parse_property(&bytes),
            Some(Property {
                name: 0x18068b8c8,
                kind: 0,
                child: 0x18093b000,
                offset: 0xd8,
                inside_array: false
            })
        );
        assert_eq!(parse_property(&bytes[..72]), None);
        bytes[72..76].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(parse_property(&bytes), None);
        bytes[72..76].copy_from_slice(&i32::MAX.to_le_bytes());
        assert_eq!(parse_property(&bytes), None);
    }
}
