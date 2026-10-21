//! XEX packaging for the existing linker.
//! Address placement and overlap policy belong to the linker's existing `place` layer.

use atari_media::{Segment, Xex};

/// Inspects ordered segments without executing INITAD/RUNAD or collapsing overlaps.
pub fn inspect(bytes: &[u8]) -> Result<Vec<Segment>, String> {
    Ok(Xex::parse(bytes)?.segments)
}

/// Emits already-placed segments. No implicit vectors, loader code, or gap filling.
pub fn create(segments: Vec<Segment>) -> Result<Vec<u8>, String> {
    Xex { segments }.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xex_packaging_keeps_disjoint_and_repeated_segments() {
        let segments = vec![
            Segment {
                start: 0x2000,
                bytes: vec![0xa9, 42, 0x60],
            },
            Segment {
                start: 0x4000,
                bytes: vec![1, 2],
            },
            Segment {
                start: 0x2000,
                bytes: vec![0xea],
            },
        ];
        let bytes = create(segments.clone()).unwrap();
        assert_eq!(inspect(&bytes).unwrap(), segments);
        assert_eq!(&bytes[..6], &[255, 255, 0, 32, 2, 32]);
        assert!(inspect(&bytes[..bytes.len() - 1]).is_err());
        assert!(
            create(vec![Segment {
                start: 0xfffe,
                bytes: vec![0; 3]
            }])
            .is_err()
        );
    }
}
