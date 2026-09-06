//! 文件名自然排序(natural sort)。
//!
//! 规则(需求 §10/§14):
//! - 连续 ASCII 数字段按数值比较("2" < "10");
//! - 非数字段按 Unicode 码点逐字符比较(先忽略大小写,再区分大小写,保证全序确定);
//! - 数值相等时,前导零更少的排前面("2.jpg" < "02.jpg");
//! - 数字段与非数字段比较时按首字符码点,与普通字典序一致;
//! - 只处理 ASCII 数字;全角数字等按普通字符处理(确定性)。

use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    /// 原始数字串(可能含前导零)。
    Num(String),
    /// 非数字文本段。
    Txt(Vec<char>),
}

fn tokenize(s: &str) -> Vec<Tok> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut num = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    num.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            toks.push(Tok::Num(num));
        } else {
            let mut txt = Vec::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    break;
                }
                txt.push(d);
                chars.next();
            }
            toks.push(Tok::Txt(txt));
        }
    }
    toks
}

fn lower_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// 比较两个等值数字串:先去前导零比长度,再逐位比较;都相等则前导零少者在前。
fn cmp_numeric(a: &str, b: &str) -> Ordering {
    let sa = a.trim_start_matches('0');
    let sb = b.trim_start_matches('0');
    sa.len()
        .cmp(&sb.len())
        .then_with(|| sa.cmp(sb))
        .then_with(|| a.len().cmp(&b.len()))
}

fn cmp_tok(x: &Tok, y: &Tok) -> Ordering {
    match (x, y) {
        (Tok::Num(a), Tok::Num(b)) => cmp_numeric(a, b),
        (Tok::Txt(a), Tok::Txt(b)) => {
            let la: Vec<char> = a.iter().copied().map(lower_char).collect();
            let lb: Vec<char> = b.iter().copied().map(lower_char).collect();
            la.cmp(&lb).then_with(|| a.cmp(b))
        }
        // 数字段 vs 文本段:按首字符码点(数字 '0'-'9' 与文本首字符)
        (Tok::Num(a), Tok::Txt(b)) => a.chars().next().unwrap_or('0').cmp(&lower_char(b[0])),
        (Tok::Txt(a), Tok::Num(b)) => lower_char(a[0]).cmp(&b.chars().next().unwrap_or('0')),
    }
}

/// 对完整文件名(含扩展名)进行自然排序比较。全序、确定。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let ta = tokenize(a);
    let tb = tokenize(b);
    for (x, y) in ta.iter().zip(tb.iter()) {
        match cmp_tok(x, y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    ta.len().cmp(&tb.len())
}

/// 对文件名切片按自然排序原地排序。
pub fn sort_names<T, F>(items: &mut [T], name_of: F)
where
    F: Fn(&T) -> &str,
{
    items.sort_by(|a, b| natural_cmp(name_of(a), name_of(b)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted<'a>(names: &[&'a str]) -> Vec<&'a str> {
        let mut v: Vec<&str> = names.to_vec();
        sort_names(&mut v, |s| s);
        v
    }

    #[test]
    fn numeric_order() {
        assert_eq!(
            sorted(&["10.jpg", "2.jpg", "1.jpg", "20.jpg"]),
            vec!["1.jpg", "2.jpg", "10.jpg", "20.jpg"]
        );
        assert_eq!(
            sorted(&["IMG_10.jpg", "IMG_2.jpg", "IMG_1.jpg"]),
            vec!["IMG_1.jpg", "IMG_2.jpg", "IMG_10.jpg"]
        );
        assert_eq!(
            sorted(&["100.jpg", "10.jpg", "1.jpg"]),
            vec!["1.jpg", "10.jpg", "100.jpg"]
        );
    }

    #[test]
    fn leading_zeros() {
        // 数值相等时,前导零少的在前;数值不同时正常比较
        assert_eq!(
            sorted(&["002.jpg", "2.jpg", "1.jpg", "03.jpg", "003.jpg"]),
            vec!["1.jpg", "2.jpg", "002.jpg", "03.jpg", "003.jpg"]
        );
    }

    #[test]
    fn case_insensitive_text() {
        // 忽略大小写比较;忽略大小写后相等时按原始码点(大写在前),
        // 保证全序确定
        assert_eq!(
            sorted(&["BANANA.jpg", "apple.jpg", "Apple.jpg"]),
            vec!["Apple.jpg", "apple.jpg", "BANANA.jpg"]
        );
    }

    #[test]
    fn unicode_and_emoji() {
        assert_eq!(
            sorted(&["文件10.txt", "文件2.txt", "文件1.txt"]),
            vec!["文件1.txt", "文件2.txt", "文件10.txt"]
        );
        // emoji 按码点,确定性即可
        let mut v = vec!["🍎2.jpg", "🍎1.jpg"];
        sort_names(&mut v, |s| s);
        assert_eq!(v, vec!["🍎1.jpg", "🍎2.jpg"]);
    }

    #[test]
    fn mixed_text_and_numbers() {
        assert_eq!(
            sorted(&["a10b", "a2b", "a1b", "ab", "a2a"]),
            vec!["a1b", "a2a", "a2b", "a10b", "ab"]
        );
    }

    #[test]
    fn spaces_and_special() {
        assert_eq!(
            sorted(&["file 10.jpg", "file 2.jpg", "file.jpg"]),
            vec!["file 2.jpg", "file 10.jpg", "file.jpg"]
        );
    }

    #[test]
    fn total_order_deterministic() {
        // 同名文件在文件系统上不可能出现,但比较器必须是全序
        let names = vec!["a.jpg", "A.jpg", "1.jpg", "01.jpg", "á.jpg", "z", "Z"];
        let r1 = sorted(&names);
        let r2 = sorted(&names);
        assert_eq!(r1, r2);
    }

    #[test]
    fn big_numbers_no_overflow() {
        // 100 位数字,不能溢出 u64
        let big = "1".repeat(100);
        let bigger = "2".repeat(100);
        let mut v = vec![bigger.as_str(), big.as_str()];
        sort_names(&mut v, |s| s);
        assert_eq!(v, vec![big.as_str(), bigger.as_str()]);
    }
}
