const { test } = require('node:test');
const assert = require('node:assert/strict');
const { countLines, limitFor, overBudget } = require('./check-module-size.cjs');

test('line counts support empty files, CRLF and a missing final newline', () => {
  assert.equal(countLines(''), 0);
  assert.equal(countLines('one'), 1);
  assert.equal(countLines('one\n'), 1);
  assert.equal(countLines('one\r\ntwo\r\n'), 2);
  assert.equal(countLines('one\ntwo'), 2);
});

test('new modules fail only when exceeding the implementation budget', () => {
  const file = 'mochi-app/src/app/new_feature.rs';
  assert.equal(limitFor(file), 1500);
  assert.deepEqual(overBudget([{ file, lines: 1500 }]), []);
  assert.equal(overBudget([{ file, lines: 1501 }]).length, 1);
});

test('facades have a tighter budget than implementation modules', () => {
  assert.equal(limitFor('mochi-app/src/app.rs'), 800);
  assert.equal(limitFor('mochi-app/src/ui/document.rs'), 200);
  assert.equal(overBudget([{ file: 'mochi-app/src/ui/document.rs', lines: 201 }]).length, 1);
});

test('the existing oversized file cannot grow under its legacy allowance', () => {
  const file = 'mochi-app/src/app/file_approvals.rs';
  assert.deepEqual(overBudget([{ file, lines: 1667 }]), []);
  assert.equal(overBudget([{ file, lines: 1668 }]).length, 1);
});
