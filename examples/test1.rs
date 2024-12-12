use kcov_rs::open;

fn main() {
    let mut handle = open();
    let result = handle.collect(|| {
        println!("Hello world!");
    });

    eprintln!("{result:#?}");
}
