const { test } = require('node:test');
const assert = require('node:assert/strict');
const { releasePolicy, validateVersions } = require('../scripts/release-policy.cjs');
test('una prerelease nunca usa latest y el tag debe coincidir exactamente', () => {
  assert.equal(releasePolicy('1.2.3', 'v1.2.3').npm_tag, 'latest');
  assert.deepEqual(releasePolicy('0.3.0-beta.1', 'v0.3.0-beta.1'), { version: '0.3.0-beta.1', npm_tag: 'next', prerelease: true });
  for (const tag of ['1.2.3', 'v1.2.4', 'vv1.2.3', 'v1.2.3\n']) assert.throws(() => releasePolicy('1.2.3', tag));
  for (const version of ['1.2', '01.2.3', '1.2.3-01', '1.2.3/evil']) assert.throws(() => releasePolicy(version));
});
test('el instalador, el ejecutable y npm declaran la misma versión', () => {
  assert.equal(validateVersions(), require('../package.json').version);
});
