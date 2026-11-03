import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserFiles } from './filesystem.mjs';
const blob = bytes => ({size:bytes.length,slice:(start,end) => bytes.slice(start,end)});
const create = () => new BrowserFiles(bytes => bytes);
test('case-insensitive mounted files support bounded reads without full archive copies', () => {
  const fs=create(); const bytes=new Uint8Array([10,20,30,40]);
  fs.mount('Data/PACK/Test.cpz',blob(bytes));
  assert.equal(fs.size('/GAME/data/pack/test.cpz'),4);
  assert.equal(fs.size('/game/data/pack'),-2);
  assert.deepEqual(fs.read('/game/data/pack/test.cpz',1,2),new Uint8Array([20,30]));
  assert.equal(fs.read('/game/data/pack/test.cpz',100,2).length,0);
  assert.equal(fs.size('/game/missing'),-1);
  assert.throws(() => fs.mount('data/pack/test.CPZ',blob(bytes)),/Duplicate/);
});
test('game files and traversal paths cannot be mutated', () => {
  const fs=create(); fs.mount('file',blob(new Uint8Array([1])));
  for (const op of ['truncate','remove','rename']) assert.throws(() => fs.operation(op,'/game/file','/user/x'),/read only/);
  for (const path of ['/user/../game/file','/user/./file','/user/C:/x','/user\\x']) assert.throws(() => fs.operation('truncate',path,''),/Invalid/);
  assert.deepEqual(fs.read('/game/file',0,1),new Uint8Array([1]));
});
test('staged replacement, backups and restored saves preserve bytes and exclude locks', () => {
  const fs=create(); fs.operation('truncate','/user/.save.tmp','');
  fs.write('/user/.save.tmp',2,new Uint8Array([3,4]));
  fs.operation('rename','/user/.save.tmp','/user/save001.dat');
  fs.operation('truncate','/user/.cmvs-system.lock','');
  assert.equal(fs.snapshot().length,1);
  const next=create(); next.restore(fs.snapshot());
  assert.deepEqual(next.read('/user/save001.dat',0,4),new Uint8Array([0,0,3,4]));
  assert.throws(() => next.restore([['/game/file',new Uint8Array()]]),/read only/);
  assert.deepEqual(next.read('/user/save001.dat',0,4),new Uint8Array([0,0,3,4]));
  next.operation('remove','/user/save001.dat',''); assert.equal(next.size('/user/save001.dat'),-1);
});
