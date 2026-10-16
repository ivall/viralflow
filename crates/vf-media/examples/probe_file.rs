//! Prints what ViralFlow reads from a media file and times decoding a frame.
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("path"));
    let info = vf_media::probe::probe(&path);
    println!("{info:#?}");
    let started = std::time::Instant::now();
    match vf_media::video::VideoDecoder::open(&path, 540, 960) {
        Ok(mut decoder) => {
            let frame = decoder.frame_at(1.0);
            println!("frame: {:?} in {:?}", frame.map(|f| f.map(|f| (f.width, f.height))), started.elapsed());
        }
        Err(error) => println!("open failed: {error}"),
    }
}
