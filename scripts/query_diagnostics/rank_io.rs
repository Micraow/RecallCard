// 标准库二进制协议适配；排名与分词函数在编译前从生产文件原样提取。
// 请求：MAGIC，query/mode 两个 UTF-8 帧，u32 文档数，然后逐个 Document。
// 文档：reference/text，entities/labels 两个字符串向量，evidence/state。
// 响应：MAGIC，实际 ranking_terms 向量，u32 命中数，逐项 u32 索引/f64 分数。
// 所有整数、浮点数字节均为 little-endian；每个字符串由 u32 字节长度前缀。
use std::io::{self, Cursor, Read, Write};

const MAGIC: &[u8; 8] = b"RCRANK1\0";
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_ITEMS: usize = 1_000_000;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read_u32(input: &mut Cursor<&[u8]>) -> io::Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_count(input: &mut Cursor<&[u8]>) -> io::Result<usize> {
    let value = read_u32(input)? as usize;
    if value > MAX_ITEMS {
        return Err(invalid("条目数量超过协议限制"));
    }
    Ok(value)
}

fn read_string(input: &mut Cursor<&[u8]>) -> io::Result<String> {
    let length = read_u32(input)? as usize;
    if length > MAX_BYTES || length > input.get_ref().len() - input.position() as usize {
        return Err(invalid("UTF-8 帧超限或截断"));
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| invalid("帧不是有效 UTF-8"))
}

fn read_strings(input: &mut Cursor<&[u8]>) -> io::Result<Vec<String>> {
    let count = read_count(input)?;
    (0..count).map(|_| read_string(input)).collect()
}

fn write_u32(output: &mut Vec<u8>, value: usize) -> io::Result<()> {
    let value = u32::try_from(value).map_err(|_| invalid("响应长度超过 u32"))?;
    output.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_string(output: &mut Vec<u8>, value: &str) -> io::Result<()> {
    write_u32(output, value.len())?;
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn replay() -> io::Result<()> {
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid("请求超过协议限制"));
    }
    let mut input = Cursor::new(bytes.as_slice());
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("排名回放协议不匹配"));
    }
    let query = read_string(&mut input)?;
    let mode = read_string(&mut input)?;
    if !matches!(
        mode.as_str(),
        "exact" | "identifier_prefix" | "natural_question"
    ) {
        return Err(invalid("不支持的 lexical_match"));
    }
    let count = read_count(&mut input)?;
    let mut documents = Vec::new();
    let mut ids = BTreeSet::new();
    for _ in 0..count {
        let reference = read_string(&mut input)?;
        if !ids.insert(reference.clone()) {
            return Err(invalid("source_id 必须唯一"));
        }
        documents.push(Document {
            reference,
            text: read_string(&mut input)?,
            entities: read_strings(&mut input)?,
            labels: read_strings(&mut input)?,
            evidence: read_string(&mut input)?,
            state: read_string(&mut input)?,
        });
    }
    if input.position() as usize != bytes.len() {
        return Err(invalid("请求包含多余字节"));
    }
    let terms = search::ranking_terms(&query, &mode);
    // 诊断把空词记为无候选（完整 API 可返回错误）；rank 本身会有空串短语加分。
    let mut ranked = if terms.is_empty() {
        Vec::new()
    } else {
        rank(&documents, &terms, &query, &mode)
    };
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.reference.cmp(&b.1.reference)));
    let indices = documents
        .iter()
        .enumerate()
        .map(|(index, document)| (document.reference.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    write_u32(&mut output, terms.len())?;
    for term in &terms {
        write_string(&mut output, term)?;
    }
    write_u32(&mut output, ranked.len())?;
    for (score, document) in ranked {
        if !score.is_finite() || score <= 0.0 {
            return Err(invalid("排名核返回无效分数"));
        }
        write_u32(&mut output, indices[document.reference.as_str()])?;
        output.extend_from_slice(&score.to_le_bytes());
    }
    io::stdout().lock().write_all(&output)
}

fn main() {
    if let Err(error) = replay() {
        eprintln!("排名核回放失败：{error}");
        std::process::exit(1);
    }
}
