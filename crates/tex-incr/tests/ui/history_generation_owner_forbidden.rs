fn forge(
    record: tex_incr::BoundaryRecord,
    owner: tex_exec::RetainedEngineGeneration,
) -> tex_incr::BoundaryRecord {
    tex_incr::BoundaryRecord { owner, ..record }
}

fn main() {}
