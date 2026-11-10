import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const expoRequire = createRequire(require.resolve('expo/package.json'));
const xcode = expoRequire('xcode');
const projectPath = new URL('../ios/fractal.xcodeproj/project.pbxproj', import.meta.url);
const project = xcode.project(projectPath.pathname);
project.parseSync();
const objects = project.hash.project.objects;
const unquote = value => value?.replace(/^"|"$/g, '');
const target = Object.values(objects.PBXNativeTarget).find(value =>
  typeof value === 'object' && unquote(value.name) === 'FractalTestControl');
assert.ok(target, 'Generated test target is missing');
assert.equal(unquote(target.productType), 'com.apple.product-type.bundle.ui-testing');
const product = objects.PBXFileReference[target.productReference];
assert.equal(unquote(product.path), 'FractalTestControl.xctest');
assert.equal(unquote(product.sourceTree), 'BUILT_PRODUCTS_DIR');
assert.equal(unquote(product.explicitFileType ?? product.lastKnownFileType), 'wrapper.cfbundle');
assert.equal(readFileSync(projectPath, 'utf8').includes('.xctest.mdimporter'), false);
for (const ref of objects.XCConfigurationList[target.buildConfigurationList].buildConfigurations) {
  assert.equal(objects.XCBuildConfiguration[ref.value].buildSettings.PRODUCT_BUNDLE_IDENTIFIER,
    'com.compoundingtech.fractal.test');
}
console.log('PASS generated UI-test product is FractalTestControl.xctest; UI-testing host identity base is com.compoundingtech.fractal.test');
