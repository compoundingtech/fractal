import assert from 'node:assert/strict';
import { clientName } from '@fractal/views/clientName';

assert.equal(clientName('fractal-ios', '1.2.0', undefined), 'fractal-ios 1.2.0');
assert.equal(clientName('fractal-ios', '1.2.0', 'a0c135e3'), 'fractal-ios 1.2.0 (a0c135e3)');

assert.equal(clientName('fractal-ide', '2.0.0'), 'fractal-ide 2.0.0');
