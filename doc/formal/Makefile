.PHONY: all pdf check example clean
all: check pdf
check:
	python3 check.py
pdf:
	latexmk -pdf -interaction=nonstopmode -halt-on-error -outdir=build compiler.tex
example:
	cargo run --manifest-path example/Cargo.toml --locked
	cargo run --manifest-path example/Cargo.toml --locked --bin argument_placement
clean:
	latexmk -C -outdir=build compiler.tex
