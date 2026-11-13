//! 子进程输出与本地文本文件的解码。
//!
//! 中文 Windows 上，app 用 `CREATE_NO_WINDOW` 拉起的 cmd / PowerShell 会分配隐藏控制台，
//! 控制台代码页是系统 OEM 代码页（简体中文为 936 / GBK），输出并不是 UTF-8；
//! 记事本旧版、Excel 导出的 CSV 等本地文本也常是 ANSI 代码页（同样是 GBK）。
//! 一律 `String::from_utf8_lossy` 会把中文变成 `�`：命令报错看不懂、窗口标题乱码、
//! 快捷方式目标路径被存成乱码导致 open_app 永远打不开。
//!
//! 策略：先按严格 UTF-8 解码（UTF-8 输出、纯 ASCII 都走这条）；失败再按系统代码页解码。
//! GBK 中文恰好是合法 UTF-8 的概率极低，所以这个顺序几乎不会误判。

/// 解码子进程（cmd / PowerShell / 控制台程序）的输出：UTF-8 优先，失败按 OEM 代码页。
pub fn decode_process_output(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => decode_with_code_page(bytes, CodePage::Oem)
            .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// 解码本地文本文件：去掉 UTF-8 BOM；不是 UTF-8 时按 ANSI 代码页解码。
/// 返回 (文本, 是否按本地代码页解码)。
pub fn decode_text_file(bytes: &[u8]) -> (String, bool) {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), false),
        Err(_) => match decode_with_code_page(bytes, CodePage::Ansi) {
            Some(text) => (text, true),
            None => (String::from_utf8_lossy(bytes).into_owned(), false),
        },
    }
}

#[derive(Clone, Copy)]
enum CodePage {
    Oem,
    Ansi,
}

#[cfg(windows)]
fn decode_with_code_page(bytes: &[u8], page: CodePage) -> Option<String> {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP, CP_OEMCP};
    if bytes.is_empty() {
        return Some(String::new());
    }
    let code_page = match page {
        CodePage::Oem => CP_OEMCP,
        CodePage::Ansi => CP_ACP,
    };
    let len = i32::try_from(bytes.len()).ok()?;
    // SAFETY: 输入/输出缓冲区长度都显式传入；第一次调用只查询所需长度。
    unsafe {
        let needed = MultiByteToWideChar(code_page, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if needed <= 0 {
            return None;
        }
        let mut wide = vec![0u16; needed as usize];
        let written =
            MultiByteToWideChar(code_page, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), needed);
        if written <= 0 {
            return None;
        }
        wide.truncate(written as usize);
        Some(String::from_utf16_lossy(&wide))
    }
}

#[cfg(not(windows))]
fn decode_with_code_page(_bytes: &[u8], _page: CodePage) -> Option<String> {
    None
}

/// PowerShell 脚本前缀：让 stdout 以 UTF-8 输出（隐藏控制台默认是 OEM 代码页）。
pub const POWERSHELL_UTF8_PREAMBLE: &str =
    "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_ascii_pass_through() {
        assert_eq!(decode_process_output("你好 hello".as_bytes()), "你好 hello");
        assert_eq!(decode_process_output(b""), "");
    }

    #[test]
    fn strips_utf8_bom_for_text_files() {
        let (text, legacy) = decode_text_file(b"\xEF\xBB\xBFabc");
        assert_eq!(text, "abc");
        assert!(!legacy);
    }

    /// 只在系统 OEM 代码页为 936（简体中文 Windows）时断言 GBK 解码结果。
    #[cfg(windows)]
    #[test]
    #[allow(invalid_from_utf8)] // 故意用非 UTF-8 字节验证回退路径
    fn gbk_bytes_decode_with_the_system_code_page() {
        use windows_sys::Win32::Globalization::GetOEMCP;
        // "微信" 的 GBK 编码；按 UTF-8 解码会变成 "΢��"。
        let gbk = [0xCE, 0xA2, 0xD0, 0xC5];
        assert!(std::str::from_utf8(&gbk).is_err());
        if unsafe { GetOEMCP() } == 936 {
            assert_eq!(decode_process_output(&gbk), "微信");
        }
    }
}
