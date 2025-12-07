use std::path::Path;

use crate::epub_converter::convert_epub;

#[allow(dead_code)]
pub fn example_convert() {
    let path = Path::new("../assets/hpgof.epub");
    let fixed_path = Path::new("../assets/fixed/fixed.epub");
    convert_epub(path, fixed_path, true);
}
