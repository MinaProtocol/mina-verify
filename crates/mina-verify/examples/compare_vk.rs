// Diff the VerifierIndex produced from the embedded devnet KIMCHI VK (ground truth,
// verifies live blocks) vs the one produced from the devnet PICKLES VK via the ingester.
// Both are devnet => every field must match. First mismatch = the ingester bug.
//   cargo run --example compare_vk -p mina-verify -- <kimchi.json> <pickles.json>
use ark_poly::EvaluationDomain;
use mina_verify::verifier_index_auto;

fn pt(p: &poly_commitment::commitment::PolyComm<mina_curves::pasta::Pallas>) -> String {
    use ark_ec::AffineRepr;
    p.chunks
        .iter()
        .map(|c| match c.xy() {
            Some((x, y)) => format!("({x},{y})"),
            None => "INF".to_string(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn main() {
    let a_path = std::env::args().nth(1).expect("kimchi vk json");
    let b_path = std::env::args().nth(2).expect("pickles vk json");
    let a = verifier_index_auto(&std::fs::read_to_string(&a_path).unwrap()).expect("kimchi vk");
    let b = verifier_index_auto(&std::fs::read_to_string(&b_path).unwrap()).expect("pickles vk");

    let chk = |name: &str, x: String, y: String| {
        println!("{:<28} {} :: {}{}", name, x, y, if x == y { "" } else { "   <-- MISMATCH" });
    };
    chk("domain.size", format!("{}", a.domain.size()), format!("{}", b.domain.size()));
    chk("domain.group_gen", format!("{}", a.domain.group_gen), format!("{}", b.domain.group_gen));
    chk("max_poly_size", format!("{}", a.max_poly_size), format!("{}", b.max_poly_size));
    chk("zk_rows", format!("{}", a.zk_rows), format!("{}", b.zk_rows));
    chk("public", format!("{}", a.public), format!("{}", b.public));
    chk("prev_challenges", format!("{}", a.prev_challenges), format!("{}", b.prev_challenges));
    chk("endo", format!("{}", a.endo), format!("{}", b.endo));
    for i in 0..a.shift.len() {
        chk(&format!("shift[{i}]"), format!("{}", a.shift[i]), format!("{}", b.shift[i]));
    }
    for i in 0..a.sigma_comm.len() {
        chk(&format!("sigma_comm[{i}]"), pt(&a.sigma_comm[i]), pt(&b.sigma_comm[i]));
    }
    for i in 0..a.coefficients_comm.len() {
        chk(&format!("coefficients_comm[{i}]"), pt(&a.coefficients_comm[i]), pt(&b.coefficients_comm[i]));
    }
    chk("generic_comm", pt(&a.generic_comm), pt(&b.generic_comm));
    chk("psm_comm", pt(&a.psm_comm), pt(&b.psm_comm));
    chk("complete_add_comm", pt(&a.complete_add_comm), pt(&b.complete_add_comm));
    chk("mul_comm", pt(&a.mul_comm), pt(&b.mul_comm));
    chk("emul_comm", pt(&a.emul_comm), pt(&b.emul_comm));
    chk("endomul_scalar_comm", pt(&a.endomul_scalar_comm), pt(&b.endomul_scalar_comm));
}
