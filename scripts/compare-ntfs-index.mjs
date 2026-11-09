import { closeSync, openSync, writeSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';

const [legacyPath, sequentialPath, output] = process.argv.slice(2);
if (!legacyPath || !sequentialPath || !output) {
  throw new Error('Usage: node compare-ntfs-index.mjs <legacy db> <sequential db> <new differences.jsonl>');
}

const open = (path) => {
  const db = new DatabaseSync(path, { readOnly: true });
  const catalog = db.prepare(`SELECT g.id, g.phase, hex(g.journal) AS journal, g.next_usn AS usn
    FROM index_state s JOIN generations g ON g.id=s.active_generation`).get();
  if (catalog?.phase !== 'ready') throw new Error('Expected a published journal index.');
  const integrity = db.prepare('PRAGMA integrity_check').get();
  if (integrity.integrity_check !== 'ok' || db.prepare('PRAGMA foreign_key_check').get()) {
    throw new Error('Catalog failed integrity or foreign key checks.');
  }
  const statement = db.prepare(`SELECT hex(p.id) AS key, hex(p.object) AS object,
    hex(p.parent) AS parent, hex(p.name) AS utf16, o.kind, CAST(o.size AS TEXT) AS size,
    hex(o.created) AS created, hex(o.modified) AS modified, o.attributes
    FROM paths p JOIN objects o ON o.generation=p.generation AND o.id=p.object
    WHERE p.generation=? ORDER BY p.id`);
  return { db, catalog, rows: statement.iterate(catalog.id) };
};

const a = open(legacyPath);
const b = open(sequentialPath);
const result = { legacy: a.catalog, sequential: b.catalog, legacyOnly: 0, sequentialOnly: 0, matched: 0, differences: {} };
const file = openSync(output, 'wx');
try {
  let left = a.rows.next();
  let right = b.rows.next();
  while (!left.done || !right.done) {
    if (right.done || (!left.done && left.value.key < right.value.key)) {
      result.legacyOnly++;
      writeSync(file, JSON.stringify({ legacy: left.value }) + '\n');
      left = a.rows.next();
    } else if (left.done || right.value.key < left.value.key) {
      result.sequentialOnly++;
      writeSync(file, JSON.stringify({ sequential: right.value }) + '\n');
      right = b.rows.next();
    } else {
      result.matched++;
      const fields = Object.keys(left.value).filter((key) => left.value[key] !== right.value[key]);
      for (const key of fields) result.differences[key] = (result.differences[key] ?? 0) + 1;
      if (fields.length) writeSync(file, JSON.stringify({ fields, legacy: left.value, sequential: right.value }) + '\n');
      left = a.rows.next();
      right = b.rows.next();
    }
  }
  console.log(JSON.stringify(result));
} finally {
  closeSync(file);
  a.rows.return();
  b.rows.return();
  a.db.close();
  b.db.close();
}
