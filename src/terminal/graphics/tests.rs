use super::animation::*;
use super::kitty::*;
use super::sixel::*;
use super::store::*;
use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn sixel_rejects_cumulative_overpainting() {
    let attack = b"!8192~$".repeat(8192);
    assert!(decode_sixel(&attack, true).is_none());
    // Normal repeated colors and overpainting are supported.
    let image = b"#1;2;100;0;0!800~$#2;2;0;100;0!800@-".repeat(100);
    let (w, h, rgba) = decode_sixel(&image, true).unwrap();
    assert_eq!((w, h), (800, 600));
    assert_eq!(&rgba[..4], &[0, 255, 0, 255]);
    assert_eq!(&rgba[800 * 4..800 * 4 + 4], &[255, 0, 0, 255]);
}

#[test]
#[ignore = "manual performance measurement"]
fn benchmark_sixel_legitimate_image() {
    let image = b"#1;2;100;0;0!800~$#2;2;0;100;0!800@-".repeat(100);
    let start = Instant::now();
    for _ in 0..50 {
        let decoded = decode_sixel(std::hint::black_box(&image), true).unwrap();
        assert_eq!((decoded.0, decoded.1), (800, 600));
        std::hint::black_box(decoded);
    }
    eprintln!("Sixel 800x600, two color passes: {:?}/image", start.elapsed() / 50);
}

#[test]
fn base64_round_trip() {
    assert_eq!(base64_decode(b"aGVsbG8=").unwrap(), b"hello");
    assert_eq!(base64_decode(b"aGVsbG8").unwrap(), b"hello");
    assert!(base64_decode(b"a$").is_none());
}

#[test]
fn kitty_rgb_transmit_and_display() {
    let mut store = Store::default();
    // 2x1 RGB: red, blue.
    let (cmd, payload) = parse_kitty(b"Ga=T,f=24,s=2,v=1,i=7;/wAAAAD/").unwrap();
    let (out, reply) = store.kitty(cmd, payload);
    assert!(matches!(out, KittyOutcome::Place { id: 7 }));
    assert_eq!(reply.as_deref(), Some("i=7;OK"));
    let img = store.image(7).unwrap();
    assert_eq!((img.width, img.height), (2, 1));
    assert_eq!(&img.rgba[..], [255, 0, 0, 255, 0, 0, 255, 255]);

    // Quiet transmissions and missing images.
    let (cmd, p) = parse_kitty(b"Ga=t,f=24,s=1,v=1,i=8,q=1;AAAA").unwrap();
    assert!(store.kitty(cmd, p).1.is_none());
    let (cmd, p) = parse_kitty(b"Ga=p,i=99").unwrap();
    assert_eq!(store.kitty(cmd, p).1.as_deref(), Some("i=99;ENOENT:image not found"));
}

fn run(store: &mut Store, cmd: &str) -> Option<String> {
    let (c, p) = parse_kitty(cmd.as_bytes().to_vec().leak()).unwrap();
    store.kitty(c, p).1
}

#[test]
fn kitty_animation_frames_play_from_the_clock() {
    let mut store = Store::default();
    // 1x1 red root frame, then green (100 ms) and blue (50 ms) frames.
    run(&mut store, "Ga=t,f=24,s=1,v=1,i=5,q=2;/wAA");
    assert_eq!(run(&mut store, "Ga=f,f=24,i=5,z=100;AP8A").as_deref(), Some("i=5;OK"));
    run(&mut store, "Ga=f,f=24,i=5,z=50,q=2;AAD/");
    assert_eq!(run(&mut store, "Ga=f,f=24,i=5,r=9;AAD/").as_deref(), Some("i=5;EINVAL:no such frame"));
    let t0 = Instant::now();
    let (cmd, _) = parse_kitty(b"Ga=a,i=5,s=3,v=1").unwrap();
    store.kitty_animate(&cmd, t0).unwrap();
    let img = store.image(5).unwrap();
    let at = |ms: u64| img.frame_at(t0 + Duration::from_millis(ms));
    let rgb = |ms: u64| img.frame(at(ms).0).1[..3].to_vec();
    assert_eq!((at(30).0, rgb(30)), (0, vec![255, 0, 0]), "root frame for its 40 ms");
    assert_eq!((at(50).0, rgb(50)), (1, vec![0, 255, 0]));
    assert_eq!((at(150).0, rgb(150)), (2, vec![0, 0, 255]));
    assert_eq!(at(200).0, 0, "loops forever with v=1");
    assert_eq!(at(150).1, Some(t0 + Duration::from_millis(190)), "next change at the end of the cycle");

    // Two plays (v=3 = 2 loops), then it rests on the last frame.
    let (cmd, _) = parse_kitty(b"Ga=a,i=5,v=3,c=1").unwrap();
    store.kitty_animate(&cmd, t0).unwrap();
    let img = store.image(5).unwrap();
    assert_eq!(img.frame_at(t0 + Duration::from_millis(200)).0, 0, "second play");
    assert_eq!(img.frame_at(t0 + Duration::from_millis(400)), (2, None));

    // Stop shows a fixed frame.
    let (cmd, _) = parse_kitty(b"Ga=a,i=5,s=1,c=2").unwrap();
    store.kitty_animate(&cmd, t0).unwrap();
    assert_eq!(store.image(5).unwrap().frame_at(t0 + Duration::from_millis(999)), (1, None));
}

#[test]
fn kitty_frame_composes_over_a_base_frame() {
    let mut store = Store::default();
    // 2x1 transparent root, frame 2 = root with a red pixel at x=1, blended.
    run(&mut store, "Ga=t,f=32,s=2,v=1,i=6,q=2;AAAAAAAAAAA=");
    run(&mut store, "Ga=f,f=24,s=1,v=1,x=1,i=6,c=1,q=2;/wAA");
    assert_eq!(&store.image(6).unwrap().frame(1).1[..], [0, 0, 0, 0, 255, 0, 0, 255]);

    // a=c: copy that red pixel to the left of the root frame, overwriting.
    assert_eq!(run(&mut store, "Ga=c,i=6,r=2,c=1,X=1,w=1,h=1,C=1").as_deref(), Some("i=6;OK"));
    assert_eq!(&store.image(6).unwrap().rgba[..], [255, 0, 0, 255, 0, 0, 0, 0]);
    assert_eq!(run(&mut store, "Ga=c,i=6,r=2,c=1,X=1,w=2,h=1").as_deref(), Some("i=6;EINVAL:rectangle out of bounds"));
}

#[test]
fn kitty_compose_rejects_overflow_and_preserves_frames() {
    let mut store = Store::default();
    store.insert(1, 2, 2, vec![23; 16]);
    let original = Arc::clone(&store.image(1).unwrap().rgba);
    let key = store.image(1).unwrap().key;
    for field in ["X", "Y", "x", "y", "w", "h"] {
        for value in [u32::MAX, u32::MAX - 1, 3] {
            let body = format!("Ga=c,i=1,r=1,c=1,w=1,h=1,{field}={value}");
            let (cmd, payload) = parse_kitty(body.as_bytes()).unwrap();
            assert_eq!(store.kitty(cmd, payload).1.as_deref(), Some("i=1;EINVAL:rectangle out of bounds"), "{body}");
            let image = store.image(1).unwrap();
            assert_eq!(image.key, key, "failure must not mutate the frame");
            assert!(Arc::ptr_eq(&image.rgba, &original));
        }
    }
    // Coordinates exactly at the last pixel are valid; default dimensions
    // cover the full image. Overlapping copies read the original frame.
    assert_eq!(run(&mut store, "Ga=c,i=1,r=1,c=1,X=1,Y=1,x=1,y=1,w=1,h=1").as_deref(), Some("i=1;OK"));
    assert_eq!(run(&mut store, "Ga=c,i=1,r=1,c=1").as_deref(), Some("i=1;OK"));
}

#[test]
fn kitty_compose_checks_frame_byte_lengths_before_indexing() {
    let mut store = Store::default();
    // Internal dimension/data inconsistency must not turn into unchecked
    // pixel indexing, even though normal decoding validates these lengths.
    store.insert(1, u32::MAX, u32::MAX, vec![0; 4]);
    assert_eq!(run(&mut store, "Ga=c,i=1,r=1,c=1,w=1,h=1").as_deref(), Some("i=1;EINVAL:invalid frame dimensions"));
}

#[test]
fn tiny_transmit_only_images_have_an_independent_count_limit() {
    let mut store = Store::default();
    for id in 1..=100_000 {
        let cmd = KittyCommand { action: b't', format: 32, medium: b'd', width: 1, height: 1, id, quiet: 2, ..Default::default() };
        let (outcome, reply) = store.kitty(cmd, b"AAAAAA==");
        assert!(matches!(outcome, KittyOutcome::None));
        assert!(reply.is_none());
        assert!(store.images.len() <= MAX_IMAGES);
        assert_eq!(store.order.len(), store.images.len());
    }
    assert_eq!(store.images.len(), MAX_IMAGES);
    assert_eq!(store.bytes, MAX_IMAGES * 4);
    assert!(store.placements.is_empty());
    assert!(store.image(1).is_none());
    assert!(store.image(100_000).is_some());
    // Replacing an existing ID must not evict an unrelated image.
    store.insert(100_000, 1, 1, vec![1; 4]);
    assert_eq!(store.images.len(), MAX_IMAGES);
    assert!(store.image(100_000 - MAX_IMAGES as u32 + 1).is_some());
}

#[test]
fn image_count_eviction_releases_placements_and_animation_frames() {
    let mut store = Store::default();
    store.insert(1, 1, 1, vec![0; 4]);
    assert_eq!(run(&mut store, "Ga=f,f=32,i=1;AAAAAA==").as_deref(), Some("i=1;OK"));
    store.place(Placement { image_id: 1, placement_id: 1, line: 0, col: 0, cols: 1, rows: 1, src: [0, 0, 1, 1], z: 0, alt: false, fit: false });
    store.place_virtual(VirtualPlacement { image_id: 1, placement_id: 2, cols: 1, rows: 1, z: 0, alt: false });
    let root = Arc::downgrade(&store.image(1).unwrap().rgba);
    let frame = Arc::downgrade(&store.image(1).unwrap().frames[0].rgba);
    for id in 2..=MAX_IMAGES as u32 { store.insert(id, 1, 1, vec![0; 4]); }
    assert!(store.image(1).is_some(), "at the limit, no eviction");
    store.insert(MAX_IMAGES as u32 + 1, 1, 1, vec![0; 4]);
    assert_eq!(store.images.len(), MAX_IMAGES);
    assert!(store.image(1).is_none());
    assert!(store.placements.is_empty());
    assert!(store.virtuals.is_empty());
    assert!(root.upgrade().is_none());
    assert!(frame.upgrade().is_none());
    assert_eq!(store.bytes, MAX_IMAGES * 4, "eviction includes animation bytes");
}

#[test]
fn kitty_shared_memory_transmission() {
    let name = format!("/litty-test-{}", std::process::id());
    let cname = std::ffi::CString::new(name.clone()).unwrap();
    // SAFETY: plain POSIX calls on a descriptor this test owns.
    unsafe {
        let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
        assert!(fd >= 0);
        assert_eq!(libc::ftruncate(fd, 4), 0);
        let map = libc::mmap(std::ptr::null_mut(), 4, libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
        std::ptr::copy_nonoverlapping([1u8, 2, 3, 255].as_ptr(), map as *mut u8, 4);
        libc::munmap(map, 4);
        libc::close(fd);
    }
    let b64 = |s: &str| -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let b = s.as_bytes();
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..=c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        out
    };
    let mut store = Store::default();
    assert_eq!(run(&mut store, &format!("Ga=t,t=s,f=32,s=1,v=1,S=4,i=4;{}", b64(&name))).as_deref(), Some("i=4;OK"));
    assert_eq!(&store.image(4).unwrap().rgba[..], [1, 2, 3, 255]);
    // SAFETY: as above.
    let gone = unsafe { libc::shm_open(cname.as_ptr(), libc::O_RDONLY, 0) } < 0;
    assert!(gone, "unlinked after reading");
}

#[test]
fn kitty_chunked_png_and_query() {
    // A 1x1 red PNG.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let (a, b) = png.split_at(40);
    let mut store = Store::default();
    let (c1, p1) = parse_kitty(format!("Ga=t,f=100,i=3,m=1;{a}").as_bytes().to_vec().leak()).unwrap();
    assert!(store.kitty(c1, p1).1.is_none(), "no reply before the last chunk");
    let (c2, p2) = parse_kitty(format!("Gm=0;{b}").as_bytes().to_vec().leak()).unwrap();
    assert_eq!(store.kitty(c2, p2).1.as_deref(), Some("i=3;OK"));
    assert_eq!(&store.image(3).unwrap().rgba[..], [255, 0, 0, 255]);

    let (q, p) = parse_kitty(b"Ga=q,i=31,s=1,v=1,f=24;AAAA").unwrap();
    assert_eq!(store.kitty(q, p).1.as_deref(), Some("i=31;OK"));
    assert!(store.image(31).is_none(), "a query stores nothing");
}

#[test]
fn kitty_delete_and_limits() {
    let mut store = Store::default();
    let id = store.insert(5, 1, 1, vec![0; 4]);
    store.place(Placement { image_id: id, placement_id: 0, line: 0, col: 0, cols: 1, rows: 1, src: [0, 0, 1, 1], z: 0, alt: false, fit: false });
    let (d, p) = parse_kitty(b"Ga=d,d=i,i=5").unwrap();
    store.kitty(d, p);
    assert!(store.placements.is_empty() && store.image(5).is_some(), "lowercase keeps the data");
    let (d, p) = parse_kitty(b"Ga=d,d=A").unwrap();
    store.kitty(d, p);
    assert!(store.images.is_empty());
}

#[test]
fn sixel_decodes_bands_repeats_and_colors() {
    // Color 1 = pure red (RGB percent); one full sixel column, then `!3` of
    // the top pixel only, next band one pixel.
    let (w, h, rgba) = decode_sixel(b"#1;2;100;0;0#1~!3@-@", true).unwrap();
    assert!(w >= 4 && h == 12);
    let at = |x: usize, y: usize| &rgba[(y * w as usize + x) * 4..(y * w as usize + x) * 4 + 4];
    assert_eq!(at(0, 0), [255, 0, 0, 255]);
    assert_eq!(at(0, 5), [255, 0, 0, 255]);
    assert_eq!(at(3, 0), [255, 0, 0, 255]);
    assert_eq!(at(3, 1), [0, 0, 0, 0], "transparent background");
    assert_eq!(at(0, 6), [255, 0, 0, 255]);
    assert!(decode_sixel(b"", false).is_none());
    // Raster attributes size the image up front.
    let (w, h, _) = decode_sixel(b"\"1;1;8;6#0~", false).unwrap();
    assert_eq!((w, h), (8, 6));
}

#[test]
fn kitty_file_transmission_security() {
    let b64 = |s: &str| -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let b = s.as_bytes();
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..=c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        out
    };

    let mut store = Store::default();

    // 1. Temporary file (t=t) outside temp dir must be rejected with EPERM
    let fake_temp = "/etc/tty-graphics-protocol-test";
    let res = run(&mut store, &format!("Ga=t,t=t,f=24,s=1,v=1,i=10;{}", b64(fake_temp)));
    assert!(res.as_deref().unwrap_or("").contains("EPERM") || res.as_deref().unwrap_or("").contains("ENOENT"));

    // 2. Sensitive paths (t=f) like /etc/passwd or .ssh or hidden dotfiles must be rejected
    let res = run(&mut store, &format!("Ga=t,t=f,f=24,s=1,v=1,i=11;{}", b64("/etc/passwd")));
    assert_eq!(res.as_deref(), Some("i=11;EPERM:access denied"));
    let res_hidden = run(&mut store, &format!("Ga=t,t=f,f=24,s=1,v=1,i=13;{}", b64("/tmp/.env")));
    assert!(res_hidden.as_deref().unwrap_or("").contains("EPERM") || res_hidden.as_deref().unwrap_or("").contains("ENOENT"));

    // 3. Unauthorized shared memory prefix must be rejected
    let res_shm = run(&mut store, &format!("Ga=t,t=s,f=24,s=1,v=1,i=14;{}", b64("/arbitrary_shm_seg")));
    assert_eq!(res_shm.as_deref(), Some("i=14;EPERM:unauthorized shared memory prefix"));

    // 4. Temporary file inside temp dir with tty-graphics-protocol in name is allowed and unlinked
    let temp_path = std::env::temp_dir().join("tty-graphics-protocol-unittest.raw");
    std::fs::write(&temp_path, [255u8, 0, 0]).expect("write temp file");
    let path_str = temp_path.to_str().unwrap();
    let res = run(&mut store, &format!("Ga=t,t=t,f=24,s=1,v=1,i=12;{}", b64(path_str)));
    assert_eq!(res.as_deref(), Some("i=12;OK"));
    assert!(!temp_path.exists(), "temp file must be deleted after reading");
}

#[test]
fn kitty_chunked_transmission_size_limit() {
    let mut store = Store::default();
    let payload = vec![b'A'; 100];
    // Start a chunked transmission
    let (outcome, reply) = store.kitty(KittyCommand { action: b't', more: true, id: 99, ..KittyCommand::default() }, &payload);
    assert!(matches!(outcome, KittyOutcome::None));
    assert!(reply.is_none());
    assert!(store.pending.is_some());

    // Oversized chunk exceeding STORE_LIMIT
    let big_chunk = vec![b'A'; STORE_LIMIT + 1];
    let (outcome, reply) = store.kitty(KittyCommand { action: b't', more: true, ..KittyCommand::default() }, &big_chunk);
    assert!(matches!(outcome, KittyOutcome::None));
    assert_eq!(reply.as_deref(), Some("i=99;EBIG:chunked transmission exceeds size limit"));
    assert!(store.pending.is_none(), "pending must be cleared on overflow");
}

#[test]
fn kitty_chunked_transmission_abandoned_timeout() {
    let mut store = Store::default();
    let payload = vec![b'A'; 100];
    let _ = store.kitty(KittyCommand { action: b't', more: true, id: 101, ..KittyCommand::default() }, &payload);
    assert!(store.pending.is_some());

    // Artificially age the pending buffer past CHUNKED_TIMEOUT (5 seconds)
    if let Some((_, _, ref mut started)) = store.pending {
        *started = Instant::now() - Duration::from_secs(6);
    }

    // Next kitty command cleans up abandoned pending buffer
    let _ = store.kitty(KittyCommand { action: b'd', ..KittyCommand::default() }, &[]);
    assert!(store.pending.is_none(), "stale pending buffer must be dropped");
}

#[test]
fn auto_id_image_pruning() {
    let mut store = Store::default();
    let id = store.insert(0, 10, 10, vec![0; 400]);
    assert!(id >= (1 << 24), "auto id starts high");
    store.placements.push(Placement {
        image_id: id,
        placement_id: 1,
        line: 5,
        col: 0,
        cols: 2,
        rows: 2,
        src: [0, 0, 10, 10],
        z: 0,
        alt: false,
        fit: false,
    });
    assert!(store.image(id).is_some());

    // Prune before line 5: still kept
    store.prune(4);
    assert!(store.image(id).is_some());
    assert_eq!(store.placements.len(), 1);

    // Prune past line 5 + 2: placement removed and image freed
    store.prune(7);
    assert!(store.placements.is_empty());
    assert!(store.image(id).is_none(), "auto-assigned image must be removed when placements are gone");
}

#[test]
fn explicit_high_image_ids_survive_pruning() {
    let mut store = Store::default();
    let boundary_id = 1 << 24;
    let max_id = u32::MAX;
    store.insert(boundary_id, 10, 10, vec![0; 400]);
    store.insert(max_id, 10, 10, vec![0; 400]);

    let auto_id = store.insert(0, 10, 10, vec![0; 400]);
    store.placements.push(Placement {
        image_id: auto_id,
        placement_id: 1,
        line: 5,
        col: 0,
        cols: 2,
        rows: 2,
        src: [0, 0, 10, 10],
        z: 0,
        alt: false,
        fit: false,
    });

    // Pruning should only prune dead auto ids, not explicit images without placements
    store.prune_dead_auto_ids();
    assert!(store.image(boundary_id).is_some(), "explicit boundary ID 1<<24 must not be pruned");
    assert!(store.image(max_id).is_some(), "explicit u32::MAX ID must not be pruned");
    assert!(store.image(auto_id).is_some(), "auto image with placement must not be pruned");

    // When auto image placement is removed, only auto_id is pruned
    store.placements.clear();
    store.prune_dead_auto_ids();
    assert!(store.image(auto_id).is_none(), "auto image without placement must be pruned");
    assert!(store.image(boundary_id).is_some(), "explicit boundary ID must still survive");
    assert!(store.image(max_id).is_some(), "explicit u32::MAX ID must still survive");

    // Replacing an auto-assigned ID with explicit transmission:
    let auto_id_2 = store.insert(0, 10, 10, vec![0; 400]);
    assert!(store.image(auto_id_2).unwrap().auto);
    // Overwrite with explicit transmission of that same numeric ID
    store.insert(auto_id_2, 10, 10, vec![0; 400]);
    assert!(!store.image(auto_id_2).unwrap().auto, "must no longer be marked auto");
    store.prune_dead_auto_ids();
    assert!(store.image(auto_id_2).is_some(), "replaced explicit image must not be pruned");

    // Clear resets all
    store.clear();
    assert!(store.image(boundary_id).is_none());
    assert!(store.image(auto_id_2).is_none());
}

#[test]
fn decode_png_bounds_check() {
    // Build a minimal PNG header that declares dimensions exceeding STORE_LIMIT
    // 8192 x 8192 x 4 = 256MB > STORE_LIMIT (64MB)
    let mut png_data = super::png::encode_test_png(1, 1, 6, &[0, 0, 0, 255]);
    // Patch width and height in IHDR (offsets 16..20 and 20..24) to 8192
    png_data[16..20].copy_from_slice(&8192u32.to_be_bytes());
    png_data[20..24].copy_from_slice(&8192u32.to_be_bytes());
    // Recompute IHDR CRC (chunk type + data: bytes 12..29)
    let crc = super::png::crc32(&png_data[12..29]);
    png_data[29..33].copy_from_slice(&crc.to_be_bytes());

    let res = decode_png(&png_data);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "EINVAL:image too large");
}

/// 4x4 canvas whose default image (first frame) is only 2x2: the decoder
/// reports the frame size, not the canvas size (CWE-125, CWE-754).
fn sub_frame_apng() -> Vec<u8> {
    let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&4u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&4u32.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6; // RGBA
    super::png::write_png_chunk(&mut out, b"IHDR", &ihdr);
    let mut actl = [0u8; 8];
    actl[0..4].copy_from_slice(&1u32.to_be_bytes());
    super::png::write_png_chunk(&mut out, b"acTL", &actl);
    let mut fctl = [0u8; 26];
    fctl[4..8].copy_from_slice(&2u32.to_be_bytes());
    fctl[8..12].copy_from_slice(&2u32.to_be_bytes());
    super::png::write_png_chunk(&mut out, b"fcTL", &fctl);
    let mut raw = Vec::new();
    for _ in 0..2 {
        raw.push(0);
        raw.extend_from_slice(&[255; 2 * 4]);
    }
    let z = super::png::zlib_store(&raw);
    super::png::write_png_chunk(&mut out, b"IDAT", &z);
    super::png::write_png_chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn decode_png_never_returns_a_buffer_smaller_than_the_canvas() {
    match decode_png(&sub_frame_apng()) {
        Ok((w, h, rgba)) => assert_eq!(rgba.len(), (w * h * 4) as usize),
        Err(e) => assert_eq!(e, "EINVAL:bad PNG"),
    }
}

#[test]
fn decode_png_full_frame_roundtrips() {
    let out = super::png::encode_test_png(2, 1, 2, &[1, 2, 3, 4, 5, 6]);
    assert_eq!(decode_png(&out).unwrap(), (2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]));
}

/// Adam7 pass origins and steps: `(x0, y0, dx, dy)`.
const ADAM7: [(usize, usize, usize, usize); 7] =
    [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

/// Encode `samples` (one value per channel, row-major) as a PNG at any bit
/// depth, optionally Adam7-interlaced, with `extra` chunks before `IDAT`.
fn png_from_samples(
    (w, h): (usize, usize),
    depth: u8,
    color_type: u8,
    interlace: bool,
    samples: &[u16],
    extra: &[(&[u8; 4], &[u8])],
) -> Vec<u8> {
    let channels = match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => unreachable!(),
    };
    assert_eq!(samples.len(), w * h * channels);
    let passes: Vec<_> = if interlace { ADAM7.to_vec() } else { vec![(0, 0, 1, 1)] };
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in passes {
        for y in (y0..h).step_by(dy) {
            if x0 >= w {
                break;
            }
            raw.push(0);
            let (mut acc, mut bits) = (0u32, 0u32);
            for x in (x0..w).step_by(dx) {
                for &v in &samples[(y * w + x) * channels..][..channels] {
                    acc = (acc << depth) | u32::from(v);
                    bits += u32::from(depth);
                    while bits >= 8 {
                        bits -= 8;
                        raw.push((acc >> bits) as u8);
                    }
                }
            }
            if bits > 0 {
                raw.push((acc << (8 - bits)) as u8);
            }
        }
    }
    let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&(w as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(h as u32).to_be_bytes());
    ihdr[8] = depth;
    ihdr[9] = color_type;
    ihdr[12] = u8::from(interlace);
    super::png::write_png_chunk(&mut out, b"IHDR", &ihdr);
    for (ctype, data) in extra {
        super::png::write_png_chunk(&mut out, ctype, data);
    }
    super::png::write_png_chunk(&mut out, b"IDAT", &super::png::zlib_store(&raw));
    super::png::write_png_chunk(&mut out, b"IEND", &[]);
    out
}

fn rgba_of(png: &[u8]) -> Vec<[u8; 4]> {
    decode_png(png).unwrap().2.as_chunks::<4>().0.to_vec()
}

#[test]
fn decode_png_expands_palette_with_transparency() {
    let plte = [255, 0, 0, 0, 255, 0, 0, 0, 255];
    for depth in [1u8, 2, 4, 8] {
        let indices: &[u16] = if depth == 1 { &[0, 1, 1, 0] } else { &[0, 1, 2, 1] };
        let png = png_from_samples((4, 1), depth, 3, false, indices, &[(b"PLTE", &plte), (b"tRNS", &[0x80])]);
        let expect: Vec<[u8; 4]> = indices
            .iter()
            .map(|&i| {
                let c = &plte[i as usize * 3..][..3];
                [c[0], c[1], c[2], if i == 0 { 0x80 } else { 255 }]
            })
            .collect();
        assert_eq!(rgba_of(&png), expect, "depth {depth}");
    }
}

#[test]
fn decode_png_rejects_palette_errors() {
    let missing = png_from_samples((1, 1), 8, 3, false, &[0], &[]);
    assert_eq!(decode_png(&missing).unwrap_err(), "EINVAL:bad PNG");
    let bad_depth = png_from_samples((1, 1), 4, 2, false, &[1, 2, 3], &[]);
    assert_eq!(decode_png(&bad_depth).unwrap_err(), "EINVAL:bad PNG");
}

#[test]
fn decode_png_draws_out_of_range_palette_indices_black() {
    let png = png_from_samples((2, 1), 8, 3, false, &[0, 1], &[(b"PLTE", &[1, 2, 3])]);
    assert_eq!(rgba_of(&png), [[1, 2, 3, 255], [0, 0, 0, 255]]);
}

#[test]
fn decode_png_accepts_sixteen_bit_images_up_to_the_output_limit() {
    // 4000² RGBA16: 128 MB of raw samples for 64 MB of RGBA8 output.
    let side = 4000;
    let row = 1 + side * 8;
    let png = {
        let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&(side as u32).to_be_bytes());
        ihdr[4..8].copy_from_slice(&(side as u32).to_be_bytes());
        ihdr[8] = 16;
        ihdr[9] = 6;
        super::png::write_png_chunk(&mut out, b"IHDR", &ihdr);
        super::png::write_png_chunk(&mut out, b"IDAT", &super::png::zlib_compress_zeros(row * side));
        super::png::write_png_chunk(&mut out, b"IEND", &[]);
        out
    };
    let (w, h, rgba) = decode_png(&png).unwrap();
    assert_eq!((w, h, rgba.len()), (side as u32, side as u32, side * side * 4));
}

#[test]
fn decode_png_scales_low_bit_depth_gray() {
    let gray = |depth, samples: &[u16]| rgba_of(&png_from_samples((samples.len(), 1), depth, 0, false, samples, &[]));
    let px = |g: u8| [g, g, g, 255];
    assert_eq!(gray(1, &[0, 1, 1, 0, 1, 0, 0, 1, 1]), [0, 255, 255, 0, 255, 0, 0, 255, 255].map(px));
    assert_eq!(gray(2, &[0, 1, 2, 3]), [0, 85, 170, 255].map(px));
    assert_eq!(gray(4, &[0, 7, 15]), [0, 119, 255].map(px));
}

#[test]
fn decode_png_strips_sixteen_bit_samples_to_high_byte() {
    let rgba = png_from_samples((1, 1), 16, 6, false, &[0x1234, 0xABCD, 0xFFFF, 0x80FF], &[]);
    assert_eq!(rgba_of(&rgba), [[0x12, 0xAB, 0xFF, 0x80]]);
    let ga = png_from_samples((1, 1), 16, 4, false, &[0x4000, 0x0100], &[]);
    assert_eq!(rgba_of(&ga), [[0x40, 0x40, 0x40, 0x01]]);
}

#[test]
fn decode_png_applies_trns_color_key() {
    let gray8 = png_from_samples((2, 1), 8, 0, false, &[5, 6], &[(b"tRNS", &[0, 5])]);
    assert_eq!(rgba_of(&gray8), [[5, 5, 5, 0], [6, 6, 6, 255]]);
    // 16-bit keys compare the full sample, not the stripped high byte.
    let gray16 = png_from_samples((2, 1), 16, 0, false, &[0x0102, 0x0103], &[(b"tRNS", &[1, 2])]);
    assert_eq!(rgba_of(&gray16), [[1, 1, 1, 0], [1, 1, 1, 255]]);
    let gray2 = png_from_samples((2, 1), 2, 0, false, &[1, 2], &[(b"tRNS", &[0, 2])]);
    assert_eq!(rgba_of(&gray2), [[85, 85, 85, 255], [170, 170, 170, 0]]);
    let rgb = png_from_samples((2, 1), 8, 2, false, &[1, 2, 3, 1, 2, 4], &[(b"tRNS", &[0, 1, 0, 2, 0, 3])]);
    assert_eq!(rgba_of(&rgb), [[1, 2, 3, 0], [1, 2, 4, 255]]);
    // A malformed key is ignored rather than failing the whole image.
    let bad = png_from_samples((1, 1), 8, 2, false, &[1, 2, 3], &[(b"tRNS", &[0, 1])]);
    assert_eq!(rgba_of(&bad), [[1, 2, 3, 255]]);
}

#[test]
fn decode_png_deinterlaces_adam7() {
    for (w, h, depth, color_type) in [(5, 3, 8, 6), (9, 9, 1, 0), (1, 1, 8, 2), (17, 10, 16, 2), (3, 7, 4, 3)] {
        let channels = match color_type { 0 | 3 => 1, 2 => 3, _ => 4 };
        let max = if color_type == 3 { 3 } else { (1u32 << depth) - 1 };
        let samples: Vec<u16> = (0..w * h * channels).map(|i| ((i as u32 * 7919) % (max + 1)) as u16).collect();
        let plte: &[u8] = &[10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let extra: &[(&[u8; 4], &[u8])] = if color_type == 3 { &[(b"PLTE", plte)] } else { &[] };
        let plain = png_from_samples((w, h), depth, color_type, false, &samples, extra);
        let adam7 = png_from_samples((w, h), depth, color_type, true, &samples, extra);
        assert_eq!(decode_png(&adam7).unwrap(), decode_png(&plain).unwrap(), "{w}x{h} depth {depth} type {color_type}");
    }
}

#[test]
fn kitty_animation_frame_limits() {
    let mut store = Store::default();
    let id = store.insert(1, 10, 10, vec![0; 400]);

    // Frame count cap: fill up to MAX_ANIMATION_FRAMES
    let img = store.images.get_mut(&id).unwrap();
    for _ in 0..MAX_ANIMATION_FRAMES {
        img.frames.push(Frame { key: next_key(), rgba: vec![0; 400].into(), gap: 40 });
    }
    let (outcome, reply) = store.kitty(KittyCommand { action: b'f', id, ..KittyCommand::default() }, &[0; 400]);
    assert!(matches!(outcome, KittyOutcome::None));
    assert_eq!(reply.as_deref(), Some("i=1;EBIG:too many animation frames"));

    // Memory limit cap:
    // Create an image with 2048 x 2048 x 4 = 16MB per frame.
    // MAX_ANIMATION_BYTES is 32MB. Root frame takes 16MB.
    // 1 additional frame takes 16MB -> total 32MB <= 32MB.
    // 2nd additional frame would take 48MB > 32MB -> EBIG.
    let mut store2 = Store::default();
    let id2 = store2.insert(2, 2048, 2048, vec![0; 2048 * 2048 * 4]);
    let img2 = store2.images.get_mut(&id2).unwrap();
    img2.frames.push(Frame { key: next_key(), rgba: vec![0; 2048 * 2048 * 4].into(), gap: 40 });
    let (outcome2, reply2) = store2.kitty(KittyCommand { action: b'f', id: id2, ..KittyCommand::default() }, &[]);
    assert!(matches!(outcome2, KittyOutcome::None));
    assert_eq!(reply2.as_deref(), Some("i=2;EBIG:animation exceeds memory limit"));
}

#[test]
fn warm_placement_scroll_and_visible_paths_do_not_allocate() {
    use crate::test_alloc::allocations;
    let mut store = Store::default();
    let id = store.insert(1, 2, 2, vec![0; 16]);
    store.placements.reserve(16);
    let mut out = Vec::with_capacity(16);
    let at = |line| Placement {
        image_id: id,
        placement_id: 0,
        line,
        col: 0,
        cols: 2,
        rows: 2,
        src: [0, 0, 2, 2],
        z: 0,
        alt: true,
        fit: false,
    };
    let now = Instant::now();
    store.place(at(10));
    store.visible(0, 24, true, now, &mut out);
    out.clear();
    assert_eq!(allocations(|| store.place(at(12))), 0, "place");
    assert_eq!(allocations(|| store.visible(0, 24, true, now, &mut out)), 0, "visible");
    assert_eq!(out.len(), 2);
    out.clear();
    assert_eq!(allocations(|| store.scroll_alt(0, 23, 1)), 0, "scroll_alt");
    assert_eq!(allocations(|| store.scroll_alt_down(0, 23, 1)), 0, "scroll_alt_down");
    assert_eq!(allocations(|| store.prune(0)), 0, "prune");
    assert_eq!(allocations(|| store.delete_lines(100..101, true)), 0, "delete_lines");
}

#[test]
fn kitty_animation_loop_length_does_not_overflow() {
    let mut store = Store::default();
    run(&mut store, "Ga=t,f=24,s=1,v=1,i=7,q=2;/wAA");
    // 200 frames of i32::MAX ms each and the largest loop count: the total
    // play time (cycle * plays) is far above u64::MAX milliseconds.
    for _ in 0..200 {
        run(&mut store, "Ga=f,f=24,i=7,z=2147483647,q=2;AP8A");
    }
    let now = Instant::now();
    let (cmd, _) = parse_kitty(b"Ga=a,i=7,s=3,c=1,v=4294967295").unwrap();
    store.kitty_animate(&cmd, now).unwrap();
    let (frame, next) = store.image(7).unwrap().frame_at(now);
    assert_eq!(frame, 0, "still inside the first frame of the first play");
    assert_eq!(next, Some(now + Duration::from_millis(40)), "root frame keeps its 40 ms gap");
}
