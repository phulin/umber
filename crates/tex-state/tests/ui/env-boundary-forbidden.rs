use tex_state::env::Env;
use tex_state::env::banks::IntParam;

fn main() {
    let mut env = Env::new();
    let _default_env = Env::default();
    env.bump_epoch();
    env.enter_group();
    env.push_aftergroup(1);
    let _ = env.leave_group();
    env.set_count(0, 1);
    env.set_int_param(IntParam::new(0), 1);
}
