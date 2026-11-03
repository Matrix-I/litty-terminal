fn main() {
    println!("// Generated from the f32 stem gamma expression; see the equality test in raster.rs.");
    for (name, gamma) in [("NORMAL", 0.78f32), ("THICK", 0.55f32)] {
        println!("pub(super) const {name}: [u8; 256] = [");
        for v in 0..=255 { print!("{},", ((v as f32 / 255.0).powf(gamma) * 255.0 + 0.5) as u8); if v % 16 == 15 { println!(); } }
        println!("];");
    }
}
