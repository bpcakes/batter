-- Current-version reference protocol: retain the provider lower bound with its
-- outcome. Native scheduling remains Runledger-owned.
ALTER TABLE reference_delivery_effects
    ADD COLUMN retry_not_before timestamptz;
