struct Escape;

impl tex_exec::RetainedEngineOperation for Escape {
    type Output = tex_state::TokenListId<()>;

    fn run<G: 'static>(
        self,
        mut admitted: tex_exec::AdmittedEngineGeneration<'_, G>,
    ) -> Self::Output {
        admitted
            .universe()
            .allocate_token_list(&[])
            .expect("allocation")
    }
}

fn main() {}
