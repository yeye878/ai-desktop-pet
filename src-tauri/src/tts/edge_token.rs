//! Sec-MS-GEC v2 compatibility for Edge Read Aloud.
//! Protocol reference: https://github.com/rany2/edge-tts/blob/master/src/edge_tts/drm.py
//! This is a service token, not a general-purpose cryptographic hash.

const INITIAL: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
const PREFIX_STATE: [u32; 8] = [
    0xfd9fbea2, 0x91243ce7, 0xbe499094, 0xcd2e1e0a, 0x6c006f49, 0x04bd35ab, 0xfeb0a7e4, 0x03effbd2,
];
const PREFIX_WORDS: [u32; 14] = [
    0xeb24bf9e, 0x1558691f, 0xa0c09ee2, 0x0cfa32ba, 0xa622ee8c, 0xd1dd9491, 0xc0dd25a0, 0xf9e70ab6,
    0xf0c31573, 0x7058ce4e, 0x7188e606, 0x00e5ab33, 0xd258b060, 0x72766564,
];
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
const COEFFICIENTS: [(u32, u32); 14] = [
    (0xe82e91b1, 0xb32700d0),
    (0x44de24c6, 0x88dff51d),
    (0x3beb65bc, 0x29514038),
    (0xc89c5933, 0xb328cbdf),
    (0x8d1ec211, 0xd405b4d5),
    (0x314ef5d5, 0x26e55f3c),
    (0xe92491b1, 0x607801c6),
    (0xf0a5b9d5, 0x82b2c1fb),
    (0x06064cf3, 0xfa51a3da),
    (0xda79fa60, 0x207cc464),
    (0xbed20560, 0x3c3c74f3),
    (0x89f9c9f8, 0xd2d14b61),
    (0x35fa38c6, 0xbb04a2c1),
    (0xcb49e3c6, 0xc5626167),
];
fn sigma0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}
fn sigma1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}
fn expand(w: &mut [u32; 64], start: usize) {
    for i in start..64 {
        w[i] = sigma1(w[i - 2])
            .wrapping_add(w[i - 7])
            .wrapping_add(sigma0(w[i - 15]))
            .wrapping_add(w[i - 16]);
    }
}
fn compress(state: [u32; 8], w: &[u32; 64], start: usize) -> [u32; 8] {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
    for i in start..64 {
        let k = if start == 14 && i == 14 {
            0xc9dc06a7
        } else {
            K[i]
        };
        let t1 = h
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ (!e & g))
            .wrapping_add(k)
            .wrapping_add(w[i]);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
            .wrapping_add((a & b) ^ (a & c) ^ (b & c));
        [a, b, c, d, e, f, g, h] = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
    }
    [a, b, c, d, e, f, g, h]
}
fn digest(data: &[u8]) -> [u8; 32] {
    let mut w = [0u32; 64];
    w[14] = u32::from_be_bytes([0, data[0], data[1], data[2]]);
    w[15] = u32::from_be_bytes(data[3..7].try_into().unwrap());
    for i in 16..30 {
        let mut v = PREFIX_WORDS[i - 16];
        if i >= 16 {
            v = v.wrapping_add(sigma1(w[i - 2]) ^ if i == 16 { 0x000b92c0 } else { 0 });
        }
        if i >= 21 {
            v = v.wrapping_add(w[i - 7]);
        }
        if i >= 29 {
            v = v.wrapping_add(sigma0(w[i - 15]) ^ if i == 29 { 0x059c0b80 } else { 0 });
        }
        w[i] = v;
    }
    w[30] = sigma1(w[28])
        .wrapping_add(w[23])
        .wrapping_add(sigma0(w[15]))
        .wrapping_add(w[14])
        .wrapping_add(0xc9dc06a7)
        .wrapping_sub(K[14]);
    expand(&mut w, 31);
    let mut state = compress(PREFIX_STATE, &w, 14);
    for i in 0..8 {
        state[i] = state[i].wrapping_add(INITIAL[i]);
    }
    let length = 57 + data.len();
    let mut padded = data[7..].to_vec();
    padded.push(0x80);
    padded.resize(padded.len() + (119 - length % 64) % 64, 0);
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());
    for block in padded.chunks_exact(64) {
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        expand(&mut w, 16);
        let next = compress(state, &w, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(next[i]);
        }
    }
    let mut result = [0; 32];
    for (dest, word) in result.chunks_exact_mut(4).zip(state) {
        dest.copy_from_slice(&word.to_be_bytes());
    }
    result
}
fn checksum(data: &[u8], hash: &[u8; 32]) -> [u8; 8] {
    let keys = [
        u32::from_le_bytes(hash[0..4].try_into().unwrap()) | 1,
        u32::from_le_bytes(hash[4..8].try_into().unwrap()) | 1,
    ];
    let factors = keys.map(|key| key.wrapping_mul(0x82893933));
    let mut pairs: Vec<(u32, u32)> = COEFFICIENTS
        .iter()
        .enumerate()
        .map(|(i, (a, b))| (a.wrapping_mul(factors[i % 2]), *b))
        .collect();
    let mut padded = vec![0];
    padded.extend_from_slice(data);
    padded.resize(padded.len().div_ceil(8) * 8, 0);
    for (i, word) in padded.chunks_exact(4).enumerate() {
        let word = u32::from_le_bytes(word.try_into().unwrap());
        let mut term = word.wrapping_mul(keys[i % 2]);
        let mut offset =
            word.rotate_left(16)
                .wrapping_mul(if i % 2 == 0 { 0xef0569fb } else { 0xc31713db });
        if i == 0 {
            term = term.wrapping_add(0xf2f90b1au32.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(0x0b1a0000);
        }
        pairs.push((term, offset));
    }
    let (mut swap, mut last, mut state, mut acc) = (0u32, 0u32, 0u32, 0u32);
    for pair in pairs.chunks_exact(2) {
        let (x, ox) = pair[0];
        let (y, oy) = pair[1];
        let q = ox.wrapping_add(x).rotate_left(16).wrapping_mul(0x76f50f34);
        swap = swap.wrapping_add(q.rotate_left(16).wrapping_mul(0xc3efea97).wrapping_add(q));
        let q = oy.wrapping_add(y).rotate_left(16).wrapping_mul(0x1881ce3c);
        last = q.rotate_left(16).wrapping_mul(0x35bd1ec9).wrapping_add(q);
        swap = swap.wrapping_add(last);
        state = state.wrapping_mul(keys[0]).wrapping_add(x);
        for k in [0xcf98b111, 0x87085b9f, 0x12ceb96d, 0x257e1d83] {
            state = state.rotate_left(16).wrapping_mul(k);
        }
        acc = acc.wrapping_add(state);
        state = state.wrapping_mul(keys[1]).wrapping_add(y);
        for k in [0xa27416f5, 0xd38396ff, 0x7c932b89, 0xbfa49f69] {
            state = state.rotate_left(16).wrapping_mul(k);
        }
        acc = acc.wrapping_add(state);
    }
    let mut out = [0; 8];
    out[..4].copy_from_slice(&(last ^ state).to_le_bytes());
    out[4..].copy_from_slice(&(swap ^ acc).to_le_bytes());
    out
}
pub(super) fn generate(unix_seconds: u64) -> String {
    let seconds = unix_seconds + 11_644_473_600;
    let ticks = (seconds - seconds % 300) * 10_000_000;
    let data = format!("{ticks}6A5AA1D4EAFF4E9FB37E23D68491D6F4");
    let hash = digest(data.as_bytes());
    format!(
        "{}-{}",
        hex::encode_upper(checksum(data.as_bytes(), &hash)),
        hex::encode_upper(hash)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_independent_upstream_v2_vectors() {
        // Generated with upstream Python drm.generate_sec_ms_gec_v2, not this implementation.
        for (time, token) in [
            (
                0,
                "52BC7A17B50DFEC9-28C4394775C8751CD2F6859C030C06DB1489ACAB3B3A7651831FC26895177ED6",
            ),
            (
                1700000000,
                "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2",
            ),
            (
                1794528000,
                "AEFFBEA15B80ADA2-6D7525C3E2AD52D4EE1162E9FA0AB69D2F7A9B33685EF386E48FB044B87A58AA",
            ),
            (
                1794528300,
                "D013D9FFDF71A9AF-4D5CDFC7D3A31751F954D025DB775BC8D54F495B0125F73A93022F58088D1D45",
            ),
            (
                2147483647,
                "96AE288915A09C99-EB52B45DB68387A1383783E40D42BBBD69D4DCCDF2540629BD4E7FB2E3287822",
            ),
        ] {
            assert_eq!(generate(time), token, "time={time}");
        }
        assert_eq!(generate(1794528000), generate(1794528299));
        assert_ne!(generate(1794528299), generate(1794528300));
    }
}
