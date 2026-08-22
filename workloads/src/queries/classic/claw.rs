::mini_linq::workload_query! {
    pub struct ClawProgram;
    relation A(c0: i32, c1: i32);
    relation B(c0: i32, c1: i32);
    relation C(c0: i32, c1: i32);

    claw(center, a, b, c) :- A(center, a), B(center, b), C(center, c).
}
