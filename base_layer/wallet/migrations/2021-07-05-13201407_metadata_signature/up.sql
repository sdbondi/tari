-- This migration is part of a testnet reset and should not be used on db's with existing old data in them
-- thus this migration does not accommodate db's with existing rows.

PRAGMA foreign_keys=off;
DROP TABLE outputs;
CREATE TABLE outputs (
    id BIGINT NOT NULL PRIMARY KEY,
    commitment BLOB NULL,
    spending_key BLOB NOT NULL,
    value BIGINT NOT NULL,
    flags INTEGER NOT NULL,
    maturity bigint NOT NULL,
    status INTEGER NOT NULL,
    tx_id bigint NULL,
    hash BLOB NULL,
    script BLOB NOT NULL,
    input_data BLOB NOT NULL,
    script_private_key BLOB NOT NULL,
    sender_offset_public_key BLOB NOT NULL,
    metadata_signature_nonce BLOB NOT NULL,
    metadata_signature_u_key BLOB NOT NULL,
    metadata_signature_v_key BLOB NOT NULL,
    CONSTRAINT unique_commitment UNIQUE (commitment)
);
PRAGMA foreign_keys=on;
