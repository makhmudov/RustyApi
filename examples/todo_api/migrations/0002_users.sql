CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'user',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE todos ADD COLUMN owner_id BIGINT REFERENCES users (id) ON DELETE CASCADE;
CREATE INDEX todos_owner_id ON todos (owner_id);
