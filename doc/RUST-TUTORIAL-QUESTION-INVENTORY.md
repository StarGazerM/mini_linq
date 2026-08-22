# Question Inventory: The First Two Encounters

> Status: design artifact, not final dialogue.
>
> Scope: the first 30 questions of *The Little Rust Metaprogrammer*.
> Encounter 1 separates a program from its result. Encounter 2 discovers that
> an operation over a datatype owes one case to every variant.

## 1. Inventory contract

Each question records:

1. the exact object visible to the reader;
2. the question asked about that object;
3. the intended short answer;
4. a tempting wrong answer;
5. the misconception under pressure; and
6. why the next question has become necessary.

The tags are:

- **predict** — determine an exact state or result;
- **distinguish** — separate two objects or two questions;
- **construct** — produce an exact Rust value;
- **trace** — follow one operation step by step;
- **counterexample** — break a tempting generalization;
- **law** — state an invariant already supported by examples;
- **transfer** — apply the invariant to a fresh object; and
- **evidence** — use an actual Rust compiler response.

The answers should remain shorter than the questions. Explanations come only
after the reader predicts.

## 2. Exact objects

### Object A: the calculator

The calculator has a tray of numbers. The rightmost number is the top. The tray
begins empty.

```text
[]
```

`push n` places `n` at the right end. `add` replaces the two rightmost numbers
with their sum.

### Object B: the first two tapes

```text
Tape A              Tape B
------              ------
push 1              push 3
push 2
add
```

### Object C: the first instruction datatype

```rust
enum Op {
    Push(i64),
    Add,
}
```

The two tapes become ordinary Rust sequences:

```rust
let tape_a = vec![Op::Push(1), Op::Push(2), Op::Add];
let tape_b = vec![Op::Push(3)];
```

No derived traits are shown yet. They are themselves generated code and should
not compete with the first discovery.

### Object D: the first runner

```rust
fn run(tape: &[Op]) -> i64 {
    let mut stack = Vec::new();

    for op in tape {
        match op {
            Op::Push(n) => stack.push(*n),
            Op::Add => {
                let right = stack.pop().expect("add needs a right number");
                let left = stack.pop().expect("add needs a left number");
                stack.push(left + right);
            }
        }
    }

    stack.pop().expect("the tape must produce a number")
}
```

This runner deliberately assumes the tiny tapes are well formed. The
`expect` calls create an honest debt: malformed programs will later make
`Result` and a checked stage necessary.

### Object E: the first language extension

```text
Tape C
------
push 2
push 3
mul
```

We add one variant:

```rust
enum Op {
    Push(i64),
    Add,
    Mul,
}
```

but initially leave the runner's `match` unchanged.

### Object F: an instruction that needs outside information

Only after the multiplication case works do we add:

```rust
Load(String),
```

and consider:

```text
inputs x

load x
push 1
add
```

For one run, the outside input is `x = 3`.

## 3. Encounter 1 — Is a program already its answer?

### Questions 1–4: trace before naming anything

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 1 · trace | For Tape A, what is the tray after `push 1`? | `[1]`. | `1` is already the final answer. | Runtime state is being collapsed into a result. Trace one more instruction. |
| 2 · trace | What is the tray after the following `push 2`? | `[1, 2]`. | `[3]`; addition is performed without an `add`. | Placing numbers and combining them are different actions. Now execute `add`. |
| 3 · predict | What is the tray after all of Tape A? | `[3]`. | `[1, 2, 3]`; `add` is treated as another pushed value. | The first tape now has an exact result. Run the shorter tape independently. |
| 4 · predict | Starting again from `[]`, what tray does Tape B leave? | `[3]`. | It cannot run because it has no `add`. | Different instruction forms can make complete programs. Now compare outcomes. |

### Questions 5–8: make the lost information visible

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 5 · distinguish | Do Tape A and Tape B leave the same tray? | Yes. | No, because they look different. | Equality of results has been separated from written form. Compare the writing itself. |
| 6 · distinguish | Are Tape A and Tape B the same written tape? | No. | Yes, because both mean `3`. | Equal results do not imply equal programs. Now deliberately throw the programs away. |
| 7 · counterexample | If only `[3]` is retained, can we recover whether Tape A or Tape B was written? | No. | Yes, by reversing the calculation. | Running a program may discard distinctions in its syntax. Ask what must be retained. |
| 8 · construct | What must be kept to remember Tape A exactly? | `push`, `1`, `push`, `2`, and `add`, in that order. | Only the numbers `1`, `2`, and `3`. | Operands and results cannot stand in for instruction forms. Try a representation. |

### Questions 9–13: construct programs as Rust values

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 9 · counterexample | Can the integer sequence `[1, 2]` represent all of Tape A? | No; it contains no `push` or `add`. | Yes; the operations can be guessed from position. | An undifferentiated payload sequence cannot represent distinct forms honestly. Name the forms. |
| 10 · distinguish | How many instruction forms occur in Tapes A and B? | Two: `Push(number)` and `Add`. | Three: `1`, `2`, and `add`. | Payload values are not additional syntactic forms. A datatype with two variants is now justified. |
| 11 · construct | What Rust type admits exactly those two forms? | `enum Op { Push(i64), Add }`. | `String`; any text can be stored. | A Rust datatype can state which instruction shapes exist. Inspect one value of it. |
| 12 · distinguish | Is `Op::Push(3)` the integer `3`? | No; it is an `Op` carrying `3`. | Yes; the payload and the constructed value are identical. | A constructor makes syntax data without running it. One instruction is still not a tape. |
| 13 · construct | What retains several `Op` values in written order, including repeated ones? | `Vec<Op>`. | A single `Op`, or a collection that removes repeats. | A program is an ordered sequence of syntax occurrences. Construct both complete tape values. |

### Question 14: earn the first law

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 14 · law | The Rust tapes have different instruction sequences but both run to `3`. Complete: “A program value is not the value it may ____.” | Produce. | Contain. | The syntax/result distinction is now supported by traces and constructions. A separate operation must connect the two. |

### Encounter 1 gate

Before continuing, the reader must answer without new terminology:

1. What is the difference between `Op::Push(3)` and `3`?
2. What is the difference between constructing a tape and running it?
3. Can unequal tapes produce equal results?
4. What information is lost when only the result remains?

If any answer is unstable, do not introduce `match` yet.

## 4. Encounter 2 — What must an operation know?

### Questions 15–18: make interpretation necessary

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 15 · distinguish | Does constructing `tape_a` execute it? | No. | Yes; Rust has seen `Op::Add`. | Reification is being confused with interpretation. We need an explicit running operation. |
| 16 · distinguish | While Tape A runs, which object changes: the tape or another object? | A separate tray or stack changes. | The tape gradually becomes `[3]`. | Program syntax persists while runtime state evolves. State-transition rules are now needed. |
| 17 · construct | What does `Push(n)` do to the stack, and what does `Add` do? | Append `n`; replace the top two numbers with their sum. | Both append a value. | Different variants demand different behavior. The runner must determine the current form. |
| 18 · distinguish | Can the runner act without determining which `Op` it has? | No. | Yes; it can inspect whether a numeric payload is present. | Form tests cannot be replaced with payload guesses. Rust needs form-directed branching. |

### Questions 19–23: discover matching and borrowing

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 19 · construct | Which named cases should the first `match` contain so every current form's behavior is explicit? | `Push(n)` and `Add`. | `Push(n)` and a wildcard. | A default arm would hide which form receives the second behavior and conceal future forms. Examine what one case exposes. |
| 20 · distinguish | In the `Push(n)` arm, what is `n`? | The integer carried by that instruction. | The entire tape, or the final answer. | Destructuring reveals a variant's payload. Decide what may mutate. |
| 21 · distinguish | In Object D, which value changes as execution proceeds? | `stack`. | `tape` and each `op`. | Mutation belongs to execution state, not automatically to program syntax. Ask whether the tape should survive. |
| 22 · predict | After running once, should we still be able to inspect or run the same tape again? | Yes. | No; running necessarily consumes a program. | Desired reuse makes observation without ownership transfer necessary. Now read through a shared view. |
| 23 · construct | How does Object D visit the instructions in order without taking them out of the tape? | `run` receives `&[Op]`, and `for op in tape` borrows each instruction. | Clone the complete tape before running. | Borrowing now has an exact purpose. Test whether the case analysis remains complete when the language grows. |

Question 23 introduces a borrowed slice only as “a shared view of an ordered
sequence.” Full ownership vocabulary remains for Encounter 5, where a
use-after-move counterexample makes it unavoidable.

### Questions 24–26: make extension break an existing operation

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 24 · predict | Add `Mul` to `Op`, leave the runner unchanged, and compile. Does it succeed? | No. | Yes, because Tape A contains no `Mul`. | Operations are checked against all values admitted by their input type, not only current examples. Inspect the evidence. |
| 25 · evidence | What exact case does Rust say is not covered? | `&Op::Mul`. | `Op::Add`; it is the fieldless arm already present. | The compiler identifies the new obligation. The leading `&` records that the runner matched a borrowed instruction. Repair it honestly. |
| 26 · construct | What is the honest repair, and what does Tape C then produce? | Add a `Mul` arm that pops two numbers and pushes their product; Tape C produces `6`. | Add `_ => {}`; Tape C then needs no defined behavior. | Exhaustiveness requires consideration of every admitted form, though it cannot prove the behavior is correct. Transfer to a form needing different information. |

### Questions 27–30: transfer the law to outside context

| # · tag | Exact question | Intended answer | Tempting wrong answer | Pressure and next necessity |
|---|---|---|---|---|
| 27 · transfer | Add `Load(String)` to `Op`. What happens to the repaired runner's exhaustive `match`? | It becomes incomplete again. | It remains complete because the string is not yet used. | The datatype/operation obligation transfers to every new variant. Ask what behavior this variant requires. |
| 28 · distinguish | Can `Load("x")` determine its runtime number from that instruction alone? | No. | Yes; the text `"x"` is already its value. | A source name is not its runtime binding. Supply one exact environment. |
| 29 · trace | For Object F with `x = 3`, what are all stack states? | `[] -> [3] -> [3, 1] -> [4]`. | `[] -> ["x"] -> ["x", 1]`; the name itself is pushed. | Program syntax and per-run input now have visibly different roles. Group persistent program data. |
| 30 · law | Which information belongs to the reusable program, which belongs to one run, and what law did `Mul` and `Load` reveal? | Declared names and code belong to the program; `x = 3` belongs to the run; an operation over a datatype must account for every variant. | Store `3` permanently inside `Load("x")`, and handle unknown variants with `_`. | The second law is earned, and `SourceProgram { inputs, code }` now has a reason to exist. Encounter 3 can ask what the preceding instructions establish. |

Only after the answer to Question 30 is given does this definition appear:

```rust
struct SourceProgram {
    inputs: Vec<String>,
    code: Vec<Op>,
}
```

The struct is now a compression of a distinction the reader has already made,
not an unexplained container presented at the beginning.

### Encounter 2 gate

Before continuing, the reader must be able to:

1. trace every intermediate stack for Tapes A, B, C, and Object F;
2. explain why construction does not execute a tape;
3. explain why `match` has one arm per instruction form;
4. predict the compiler's objection when a new variant is added;
5. distinguish `Load("x")` from the runtime value bound to `x`; and
6. state the second law without referring only to the displayed tapes.

## 5. Exact compiler evidence

The repository toolchain is currently `rustc 1.97.1 (8bab26f4f 2026-07-14)`.
With `Mul` added and its arm omitted, its relevant diagnostic is:

```text
error[E0004]: non-exhaustive patterns: `&Op::Mul` not covered
```

The leading `&` appears because `op` is borrowed. The tutorial should show the
short diagnostic line that names the missing case; temporary file paths, line
numbers, caret widths, and help formatting are incidental.

Do not use a wildcard arm in the teaching match. It would suppress precisely
the evidence the new variant is meant to create.

The working evidence programs were compiled with warnings denied. Before
`Mul`, they print:

```text
Tape A: 3
Tape B: 3
```

After the `Mul` arm and Tape C are added, they print:

```text
Tape A: 3
Tape B: 3
Tape C: 6
```

## 6. Causal dependency audit

| Rust or PL idea | First necessary question | Why it is not introduced earlier |
|---|---:|---|
| runtime state | 1 | the reader first sees a concrete changing tray |
| syntax versus result | 5 | two exact tapes must first produce one exact result |
| enum variants | 11 | the reader has identified two forms that need an honest representation |
| variant construction | 12 | the `Op` datatype now exists |
| ordered `Vec<Op>` | 13 | multiple instructions, including repetitions, must be retained |
| interpretation | 15 | the program is retained, but its result is no longer automatic |
| mutable runtime state | 16 | running must change something other than the tape |
| `match` | 19 | two variants demand different behavior |
| destructuring | 20 | `Push` carries a value needed by its behavior |
| borrowing | 22 | the same tape is explicitly wanted after one run |
| exhaustive checking | 24 | a new variant makes an existing operation incomplete |
| external runtime context | 28 | `Load` cannot determine a number from syntax alone |
| `SourceProgram { inputs, code }` | 30 | declared names and ordered code finally need to travel together |

## 7. Deliberate debts

The first runner uses `expect`. Do not call it safe. It creates later
questions:

- What happens for `add` on an empty stack?
- What happens when a finished tape leaves zero or several numbers?
- Can a syntactically representable `Load("y")` be rejected when only `x` is
  declared?

Those questions earn `Result`, validation, and `CheckedProgram` later.

Question 23 introduces borrowing only operationally. Encounter 5 will contrast
it with a consuming transformation and an actual use-after-move error.

No query, macro, token-stream, recursive AST, `Box`, index, database, or
procedural-macro term belongs in these questions.

## 8. Drafting constraints

When the inventory becomes dialogue:

- preserve the numbered order unless a concrete dependency changes;
- never narrate metadata words such as “misconception” to the reader;
- let the reader answer before revealing Rust code;
- show no more code than the current question can explain;
- keep stack states as lists until extraction of the final scalar is asked;
- keep the two earned laws visually separate from ordinary answers; and
- stop after Question 30 rather than previewing R1.
