pub struct Template {
    context: minijinja::Value,
}

impl Template {
    pub fn new( env: &crate::Envvars ) -> Self {
        let context = minijinja::context! { env };
        Self {
            context
        }
    }
}
