use regex::Regex;
use scraper::{Html, Selector};
use std::io::prelude::*;
use std::{
    collections::HashMap,
    fs::{self},
    io::Read,
    path::{Path, PathBuf},
};
use zip::write::SimpleFileOptions;

#[derive(Clone)]
pub struct FileData {
    file_name: PathBuf,
    file_data: String,
}

#[derive(Clone)]
pub struct BinaryFileData {
    file_name: PathBuf,
    file_data: Vec<u8>,
}

pub struct EpubConverter {
    files: Option<Vec<FileData>>,
    binary_files: Option<Vec<BinaryFileData>>,
    // This is the path from the OLD file name to the NEW file data
    altered_files: HashMap<PathBuf, String>,
    pub fixed_problems: Vec<String>,
}

pub fn convert_epub(input_path: &Path, output_path: &Path, verbose: bool) {
    let mut epub_converter = EpubConverter::new();
    epub_converter.read_epub(input_path);
    epub_converter.fix_encoding();
    epub_converter.fix_body_id_link();
    epub_converter.fix_book_language();
    epub_converter.write_epub(output_path);

    if verbose {
        epub_converter.output_fixed_problems();
    }
}

impl EpubConverter {
    pub fn new() -> EpubConverter {
        EpubConverter {
            files: None,
            binary_files: None,
            altered_files: HashMap::new(),
            fixed_problems: vec![],
        }
    }

    pub fn output_fixed_problems(&self) {
        for problem in self.fixed_problems.iter() {
            println!("{}", problem);
        }
    }

    fn read_epub(&mut self, path: &Path) -> () {
        let file = fs::File::open(path).unwrap();

        let mut initial_files = zip::ZipArchive::new(file).unwrap();

        let non_binary_suffixes: Vec<&str> = vec![
            "mimetype", "html", "xhtml", "htm", "xml", "svg", "css", "opf", "ncx",
        ];

        let mut files: Vec<FileData> = vec![];
        let mut binary_files: Vec<BinaryFileData> = vec![];

        for i in 0..initial_files.len() {
            let mut file = initial_files.by_index(i).unwrap();
            if non_binary_suffixes
                .iter()
                .any(|&x| file.name().ends_with(x))
            {
                let mut buffer: String = "".to_string();
                let _ = file.read_to_string(&mut buffer);
                files.push(FileData {
                    file_name: file.enclosed_name().unwrap(),
                    file_data: buffer,
                })
            } else if !file
                .enclosed_name()
                .unwrap()
                .into_os_string()
                .into_string()
                .unwrap()
                .ends_with("/")
            {
                let mut buffer: Vec<u8> = vec![];
                let _ = file.read_to_end(&mut buffer);
                binary_files.push(BinaryFileData {
                    file_name: file.enclosed_name().unwrap(),
                    file_data: buffer,
                });
            }
        }
        self.files = Some(files);
        self.binary_files = Some(binary_files);
    }

    fn fix_encoding(&mut self) {
        let encoding = r#"<?xml version="1.0" encoding="utf-8"?>"#;
        let re = Regex::new(
            r#"(?m)<\?xml\s+version=["'][\d.]+["']\s+encoding=["'][a-zA-Z\d\-.]+["'].*?\?>"#,
        )
        .unwrap();

        // iterate over all files
        for file in self.files.as_ref().unwrap().iter() {
            let maybe_ext = file.file_name.extension().and_then(|s| s.to_str());
            if let Some(ext) = maybe_ext
                && (ext == "html" || ext == "xhtml")
            {
                let data = file.file_data.clone();
                let name = get_name_from_data(file);
                let html = data.as_str().trim_start();
                let encoding_prefixed_html = format!("{encoding}\n{html}");
                self.altered_files.insert(file.file_name.clone(), {
                    if re.is_match(html) {
                        String::from(html)
                    } else {
                        self.fixed_problems
                            .push(format!("Prefixed correct encoding to html for {name}"));
                        encoding_prefixed_html
                    }
                });
            }
        }
    }

    fn fix_body_id_link(&mut self) {
        let mut body_id_list: Vec<(String, String)> = vec![];

        for file in self.files.as_ref().unwrap().iter() {
            let maybe_ext = file.file_name.extension().and_then(|s| s.to_str());
            if let Some(ext) = maybe_ext
                && (ext == "html" || ext == "xhtml")
            {
                let base_filename = get_name_from_data(&file);
                let data = file.file_data.clone();
                let html = data.as_str().trim_start();
                let dom = Html::parse_document(html);
                let body_selector = Selector::parse("body").unwrap();
                let element_ref = dom.select(&body_selector).next();
                if let Some(e) = element_ref
                    && let Some(id) = e.value().id()
                {
                    let link_target = format!("{} + # + {}", base_filename, id,);
                    body_id_list.push((link_target, base_filename));
                }
            }
        }

        for file in self.files.as_ref().unwrap().iter() {
            for (src, target) in body_id_list.iter() {
                if file.file_data.contains(src) {
                    let file_name_as_str = get_name_from_data(&file);
                    let fixed_data = file.file_data.replace(src, target);
                    self.altered_files
                        .insert(file.file_name.clone(), fixed_data);
                    self.fixed_problems.push(String::from(format!(
                        "Replaced link target with {src} in {target} in file {file_name_as_str}"
                    )));
                }
            }
        }
    }

    fn fix_book_language(&mut self) {
        // TODO(bparruck): Add support for language configuration further up the API layer.
        let preferred_language = "en";
        let supported_languages = vec!["en"];

        // Find the OPF file
        let container_file_data = get_file_from_files(&self.files, "META-INF/container.xml")
            .expect("Cannot find META-INF/container.xml")
            .file_data
            .clone();

        let container_dom = Html::parse_document(&container_file_data);
        let selector = Selector::parse("rootfile").unwrap();
        let element_ref = container_dom.select(&selector).next();

        let mut opf_filename: Option<&str> = None;

        // TODO(bparruck): Do we _have_ to iterate over all rootfiles? Surely there is only one?
        if let Some(e_ref) = element_ref {
            let e = e_ref.value();
            if let Some(v) = e.attr("media-type")
                && v == "application/oebps-package+xml"
            {
                opf_filename = e.attr("full-path");
            }
        }

        let opf_file =
            get_file_from_files(&self.files, opf_filename.expect("Cannot find OPF file")).unwrap();

        let opf_dom = Html::parse_document(
            &opf_file
                .file_data
                .clone()
                .replace("dc:language", "dclanguage"),
        );
        let lang_selector = Selector::parse("dclanguage").unwrap();
        let metadata_selector = Selector::parse("metadata").unwrap();

        let language_tag = opf_dom.select(&lang_selector).next();
        let metadata_tag = opf_dom
            .select(&metadata_selector)
            .next()
            .expect("Metadata tag missing in opf file");

        if let Some(lt) = language_tag {
            let mut language = lt.inner_html();
            let original_language = lt.inner_html();
            if !supported_languages.contains(&language.as_str()) {
                language = preferred_language.to_string();
                let string_to_replace = format!("<dc:language>{original_language}</dc:language>");
                let new_string = format!("<dc:language>{language}</dc:language>");
                let fixed_file = opf_file.file_data.replace(&string_to_replace, &new_string);
                self.altered_files
                    .insert(opf_file.file_name.clone(), fixed_file);
                self.fixed_problems.push(format!(
                    "Fixed language to be supported language from {original_language}"
                ));
            }
        } else {
            let metadata_html = metadata_tag.inner_html();
            let fixed_html =
                format!("{metadata_html}\n<dc:language>{preferred_language}</dc:language>");
            let fixed_file = opf_file.file_data.replace(&metadata_html, &fixed_html);
            self.altered_files
                .insert(opf_file.file_name.clone(), fixed_file);
            self.fixed_problems
                .push("Added missing language tag".to_string());
        }
    }

    fn write_epub(&mut self, path: &Path) {
        let files = self.files.as_ref().unwrap();
        let binary_files = self.binary_files.as_ref().unwrap();
        let output_file = std::fs::File::create(path).unwrap();
        let mut zip_writer = zip::ZipWriter::new(output_file);
        let options = SimpleFileOptions::default();
        for file in files.iter() {
            zip_writer
                .start_file_from_path(file.file_name.clone(), options)
                .unwrap();
            if self.altered_files.contains_key(&file.file_name) {
                zip_writer
                    .write_all(self.altered_files.get(&file.file_name).unwrap().as_bytes())
                    .unwrap();
            } else {
                zip_writer.write_all(file.file_data.as_bytes()).unwrap();
            }
        }
        for file in binary_files.iter() {
            zip_writer
                .start_file_from_path(file.file_name.clone(), options)
                .unwrap();
            zip_writer.write_all(&file.file_data).unwrap();
        }

        zip_writer.finish().unwrap();
    }
}

fn get_name_from_data(file: &FileData) -> String {
    file.file_name
        .clone()
        .into_os_string()
        .into_string()
        .unwrap()
}

fn get_file_from_files(files: &Option<Vec<FileData>>, file_name: &str) -> Option<FileData> {
    files
        .as_ref()
        .unwrap()
        .iter()
        .find(|f| get_name_from_data(f) == file_name)
        .cloned()
}
