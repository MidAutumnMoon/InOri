fn main() {

    use ino_result::ResultExt;
    use linkeepr::*;

    ino_tracing::init_tracing_subscriber();

    let cliopts = CliOpts::new().unwrap_print_error();
    let env = Envvars::new().unwrap_print_error();

    App::run_with( cliopts, env )
        .unwrap_print_error()
    ;

}
