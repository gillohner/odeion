-- Which version of identification's rules last checked a song (crate::identify::RULES_VERSION):
-- songs left waiting for review by older rules are checked again once. Only version 2 kept an
-- input key, so a song with one was checked by version 2.
ALTER TABLE identify_state ADD COLUMN rules INTEGER;
UPDATE identify_state SET rules = 2 WHERE input_key IS NOT NULL;
