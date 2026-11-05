// Run from any directory after the repository's frozen pnpm installation.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const ts = require('typescript');

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'fractal-declarations-example-'));
try {
    const sdk = path.join(temporary, 'node_modules/@fractal/declarations');
    fs.mkdirSync(sdk, { recursive: true });
    fs.writeFileSync(path.join(temporary, 'package.json'), JSON.stringify({ type: 'commonjs' }));
    fs.writeFileSync(path.join(sdk, 'package.json'), JSON.stringify({ main: 'index.js', type: 'commonjs' }));
    for (const [source, output] of [
        [path.join(__dirname, '../index.ts'), path.join(sdk, 'index.js')],
        [path.join(__dirname, 'daily-review.ts'), path.join(temporary, 'daily-review.js')],
    ]) {
        fs.writeFileSync(output, ts.transpileModule(fs.readFileSync(source, 'utf8'), {
            compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
        }).outputText);
    }
    process.stdout.write(require(path.join(temporary, 'daily-review.js')).render(process.argv.includes('--mission-only')));
} finally {
    fs.rmSync(temporary, { recursive: true, force: true });
}
