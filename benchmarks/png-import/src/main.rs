#![allow(clippy::disallowed_methods)] // Standalone host-only benchmark timing and input.

use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use png::{DecodeOptions, Decoded, Limits, StreamingDecoder, UnfilterRegion, chunk};
use sha2::{Digest, Sha256};

const REPEATS: usize = 9;
const LEVEL: u32 = 1;

#[derive(Clone, Copy)]
struct Header {
    width: usize,
    height: usize,
}

struct ResultData {
    color: Vec<u8>,
    alpha: Vec<u8>,
    workspace: usize,
}

type ImportMethod = fn(&[u8]) -> Result<ResultData, String>;

fn header(bytes: &[u8]) -> Result<Header, String> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("bad header".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    if bytes[24] != 8 || bytes[25] != 6 || bytes[28] != 0 || width == 0 || height == 0 {
        return Err("not non-interlaced RGBA8".into());
    }
    Ok(Header { width, height })
}

fn idat(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut cursor = 8usize;
    let mut data = Vec::new();
    while cursor.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        let length = u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .ok_or("overflow")?;
        if end > bytes.len() {
            return Err("truncated chunk".into());
        }
        if &bytes[cursor + 4..cursor + 8] == b"IDAT" {
            data.extend_from_slice(&bytes[cursor + 8..cursor + 8 + length]);
        }
        cursor = end;
    }
    if data.is_empty() {
        Err("missing IDAT".into())
    } else {
        Ok(data)
    }
}

fn encoders() -> (ZlibEncoder<Vec<u8>>, ZlibEncoder<Vec<u8>>) {
    (
        ZlibEncoder::new(Vec::new(), Compression::new(LEVEL)),
        ZlibEncoder::new(Vec::new(), Compression::new(LEVEL)),
    )
}

fn split_filtered_row(
    row: &[u8],
    width: usize,
    color: &mut [u8],
    alpha: &mut [u8],
) -> Result<(), String> {
    if row.len() != width * 4 + 1 || row[0] > 4 {
        return Err("bad filtered row".into());
    }
    color[0] = row[0];
    alpha[0] = row[0];
    for (index, pixel) in row[1..].chunks_exact(4).enumerate() {
        color[index * 3 + 1..index * 3 + 4].copy_from_slice(&pixel[..3]);
        alpha[index + 1] = pixel[3];
    }
    Ok(())
}

fn custom(bytes: &[u8]) -> Result<ResultData, String> {
    let info = header(bytes)?;
    let row_len = info
        .width
        .checked_mul(4)
        .and_then(|v| v.checked_add(1))
        .ok_or("row")?;
    let compressed = idat(bytes)?;
    let mut decoder = ZlibDecoder::new(compressed.as_slice());
    let (mut color_encoder, mut alpha_encoder) = encoders();
    let mut row = vec![0; row_len];
    let mut color = vec![0; info.width * 3 + 1];
    let mut alpha = vec![0; info.width + 1];
    for _ in 0..info.height {
        decoder.read_exact(&mut row).map_err(|e| e.to_string())?;
        split_filtered_row(&row, info.width, &mut color, &mut alpha)?;
        color_encoder.write_all(&color).map_err(|e| e.to_string())?;
        alpha_encoder.write_all(&alpha).map_err(|e| e.to_string())?;
    }
    let mut extra = [0];
    if decoder.read(&mut extra).map_err(|e| e.to_string())? != 0 {
        return Err("overlong".into());
    }
    Ok(ResultData {
        color: color_encoder.finish().map_err(|e| e.to_string())?,
        alpha: alpha_encoder.finish().map_err(|e| e.to_string())?,
        workspace: row.len() + color.len() + alpha.len(),
    })
}

fn strict_options() -> DecodeOptions {
    let mut options = DecodeOptions::default();
    options.set_ignore_adler32(false);
    options.set_ignore_crc(false);
    options.set_skip_ancillary_crc_failures(false);
    options
}

fn reader(bytes: &[u8]) -> Result<ResultData, String> {
    let expected = header(bytes)?;
    let mut decoder = png::Decoder::new_with_options(Cursor::new(bytes), strict_options());
    decoder.set_limits(Limits { bytes: 1024 * 1024 });
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().width as usize != expected.width
        || reader.info().height as usize != expected.height
    {
        return Err("metadata mismatch".into());
    }
    let mut row = vec![0; expected.width * 4];
    let mut color = vec![0; expected.width * 3];
    let mut alpha = vec![0; expected.width];
    let (mut color_encoder, mut alpha_encoder) = encoders();
    let mut rows = 0;
    while reader
        .read_row(&mut row)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        for (index, pixel) in row.chunks_exact(4).enumerate() {
            color[index * 3..index * 3 + 3].copy_from_slice(&pixel[..3]);
            alpha[index] = pixel[3];
        }
        color_encoder.write_all(&color).map_err(|e| e.to_string())?;
        alpha_encoder.write_all(&alpha).map_err(|e| e.to_string())?;
        rows += 1;
    }
    if rows != expected.height {
        return Err("wrong row count".into());
    }
    reader.finish().map_err(|e| e.to_string())?;
    Ok(ResultData {
        color: color_encoder.finish().map_err(|e| e.to_string())?,
        alpha: alpha_encoder.finish().map_err(|e| e.to_string())?,
        workspace: 128 * 1024 + row.len() + color.len() + alpha.len(),
    })
}

#[allow(clippy::too_many_arguments)]
fn drain_rows(
    storage: &mut [u8],
    region: &mut UnfilterRegion,
    row_len: usize,
    width: usize,
    color: &mut [u8],
    alpha: &mut [u8],
    color_encoder: &mut ZlibEncoder<Vec<u8>>,
    alpha_encoder: &mut ZlibEncoder<Vec<u8>>,
) -> Result<usize, String> {
    let rows = region.available / row_len;
    for row in storage[..rows * row_len].chunks_exact(row_len) {
        split_filtered_row(row, width, color, alpha)?;
        color_encoder.write_all(color).map_err(|e| e.to_string())?;
        alpha_encoder.write_all(alpha).map_err(|e| e.to_string())?;
    }
    let consumed = rows * row_len;
    if consumed != 0 {
        storage.copy_within(consumed..region.filled, 0);
        region.available -= consumed;
        region.filled -= consumed;
    }
    Ok(rows)
}

fn streaming(bytes: &[u8]) -> Result<ResultData, String> {
    let info = header(bytes)?;
    let row_len = info.width * 4 + 1;
    let mut decoder = StreamingDecoder::new_with_options(strict_options());
    let mut region = UnfilterRegion::default();
    let mut storage = vec![0; 32 * 1024 + 8 * 1024 + row_len * 2];
    let mut color = vec![0; info.width * 3 + 1];
    let mut alpha = vec![0; info.width + 1];
    let (mut color_encoder, mut alpha_encoder) = encoders();
    let mut input = bytes;
    let mut rows = 0;
    let mut saw_end = false;
    let mut stalls = 0;
    while !input.is_empty() && !saw_end {
        let (used, event) = decoder
            .update(input, Some(&mut region.as_buf(&mut storage)))
            .map_err(|e| e.to_string())?;
        input = &input[used..];
        rows += drain_rows(
            &mut storage,
            &mut region,
            row_len,
            info.width,
            &mut color,
            &mut alpha,
            &mut color_encoder,
            &mut alpha_encoder,
        )?;
        if matches!(event, Decoded::ImageDataFlushed) {
            region.available = region.filled;
            rows += drain_rows(
                &mut storage,
                &mut region,
                row_len,
                info.width,
                &mut color,
                &mut alpha,
                &mut color_encoder,
                &mut alpha_encoder,
            )?;
        }
        saw_end = matches!(event, Decoded::ChunkComplete(kind) if kind == chunk::IEND);
        if used == 0 {
            stalls += 1;
            if stalls > 8 {
                return Err("decoder stalled".into());
            }
        } else {
            stalls = 0;
        }
    }
    if !saw_end || !input.is_empty() || rows != info.height || region.filled != 0 {
        return Err(format!(
            "incomplete: end={saw_end} input={} rows={rows} buffered={}",
            input.len(),
            region.filled
        ));
    }
    Ok(ResultData {
        color: color_encoder.finish().map_err(|e| e.to_string())?,
        alpha: alpha_encoder.finish().map_err(|e| e.to_string())?,
        workspace: storage.len() + color.len() + alpha.len(),
    })
}

fn digest(result: &ResultData) -> String {
    let mut hash = Sha256::new();
    hash.update(&result.color);
    hash.update(&result.alpha);
    format!("{:x}", hash.finalize())
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}

fn main() -> Result<(), String> {
    let args = env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("pass framework.png qualitative_2.png teaser.png".into());
    }
    let inputs = args
        .iter()
        .map(fs::read)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let methods: [(&str, ImportMethod); 3] = [
        ("custom", custom),
        ("reader", reader),
        ("streaming", streaming),
    ];
    for (name, method) in methods {
        let mut samples = Vec::new();
        let mut last = None;
        for _ in 0..REPEATS {
            let started = Instant::now();
            let results = inputs
                .iter()
                .map(|bytes| method(black_box(bytes)))
                .collect::<Result<Vec<_>, _>>()?;
            samples.push(started.elapsed());
            last = Some(results);
        }
        let results = last.unwrap();
        let output_bytes = results
            .iter()
            .map(|r| r.color.len() + r.alpha.len())
            .sum::<usize>();
        let workspace = results.iter().map(|r| r.workspace).max().unwrap();
        let hashes = results.iter().map(digest).collect::<Vec<_>>().join(",");
        println!(
            "{name} median_ms={:.3} output_bytes={output_bytes} max_workspace={workspace} hashes={hashes}",
            median(samples).as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
