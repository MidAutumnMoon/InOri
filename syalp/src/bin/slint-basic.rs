slint::slint! {

    export component MainWindow inherits Window {
        Text { text: "hello world"; color: green; }
    }

}

fn main() -> anyhow::Result<()> {

    MainWindow::new()?.run()?;

    Ok(())

}
