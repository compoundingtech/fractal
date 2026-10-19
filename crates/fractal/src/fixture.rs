// A separate, non-installed test executable shares the CLI implementation. The binary name is
// a compile-time Cargo value; renaming st3 or setting an environment variable cannot enable it.
/* sqlite-large-value: allow-for-now lv-h97d9944f6757 owner=fractal::fixture TODO=trace-statement-and-value-representation */ include!("main.rs");
