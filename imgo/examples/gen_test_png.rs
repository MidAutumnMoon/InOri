use image::ImageBuffer;
use image::Rgba;

#[expect(clippy::unwrap_used)]
fn main() {
    let (w, h) = (37u32, 23u32);
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(w, h, |x, y| {
            Rgba([
                u8::try_from(x * 7).expect("x*7 fits u8"),
                u8::try_from(y * 11).expect("y*11 fits u8"),
                u8::try_from(x ^ y).expect("x^y fits u8"),
                255,
            ])
        });
    let out = std::env::args().nth(1).unwrap_or_else(|| "orig.png".into());
    img.save(&out).unwrap();
    eprintln!("wrote {out} ({w}x{h})");
}
