const { Client } = require("pg");

const expectedColumns = {
  sessions: ["id", "account", "created_at", "revoked_at", "revoked_reason"],
  refresh_tokens: [
    "id",
    "session_id",
    "role",
    "created_at",
    "revoked_at",
    "rotated_at",
    "replaced_by",
  ],
  revoked_tokens: ["jti", "created_at"],
};

async function main() {
  if (!process.env.DATABASE_URL) {
    throw new Error("DATABASE_URL must be set to check auth schema drift");
  }

  const client = new Client({ connectionString: process.env.DATABASE_URL });
  try {
    await client.connect();
    const { rows } = await client.query(
      `SELECT table_name, column_name
       FROM information_schema.columns
       WHERE table_schema = current_schema()
         AND table_name = ANY($1::text[])`,
      [Object.keys(expectedColumns)],
    );
    const present = new Set(rows.map(({ table_name, column_name }) => `${table_name}.${column_name}`));
    const drift = [];

    for (const [table, columns] of Object.entries(expectedColumns)) {
      for (const column of columns) {
        if (!present.has(`${table}.${column}`)) {
          drift.push(`missing column ${table}.${column}`);
        }
      }
    }

    const foreignKey = await client.query(
      `SELECT 1
       FROM information_schema.table_constraints tc
       JOIN information_schema.key_column_usage kcu
         ON tc.constraint_name = kcu.constraint_name
        AND tc.constraint_schema = kcu.constraint_schema
       JOIN information_schema.constraint_column_usage ccu
         ON ccu.constraint_name = tc.constraint_name
        AND ccu.constraint_schema = tc.constraint_schema
       WHERE tc.constraint_type = 'FOREIGN KEY'
         AND tc.table_schema = current_schema()
         AND tc.table_name = 'refresh_tokens'
         AND kcu.column_name = 'session_id'
         AND ccu.table_name = 'sessions'
         AND ccu.column_name = 'id'`,
    );
    if (foreignKey.rowCount === 0) {
      drift.push("missing foreign key refresh_tokens.session_id -> sessions.id");
    }

    if (drift.length > 0) {
      console.error("Auth schema drift detected:\n- " + drift.join("\n- "));
      process.exitCode = 1;
    } else {
      console.log("Auth database schema matches the session-management model.");
    }
  } finally {
    await client.end();
  }
}

main().catch((error) => {
  console.error("Auth schema drift check failed:", error.message);
  process.exitCode = 2;
});
