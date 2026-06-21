use image::ImageBuffer;
use image::Rgba;

fn main() {
    let (w, h) = (37u32, 23u32);
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(w, h, |x, y| {
            Rgba([(x * 7) as u8, (y * 11) as u8, (x ^ y) as u8, 255])
        });
    let out = std::env::args().nth(1).unwrap_or_else(|| "orig.png".into());
    img.save(&out).unwrap();
    eprintln!("wrote {out} ({w}x{h})");
}
