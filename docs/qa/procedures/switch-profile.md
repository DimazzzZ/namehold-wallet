# switch-profile

Make another regtest profile the active one and get its signer usable.

- **Parameters:** the target profile (a role from [environment](../environment.md#profiles)).
- **Blocks mined:** 0.

## Steps

1. On the Wallet page, pick the target profile in the profile selector of the header. Never pick a non-regtest profile ([rules](../rules.md)).
2. Wait until the Wallet page shows the target profile's balance and Owned Names.
3. If the page shows `Signer unlocked` right after the switch, press `Lock`, then `Unlock` and enter the target profile's passphrase in the secure window. A switched-to profile can show another profile's signer as unlocked, and signing then refuses with "the unlocked signer is for a different wallet profile" (a known issue, [ui](../ui.md#known-issues-not-to-report-again)).
4. Press **Sync** if the case reads chain state in the new profile.

## End state

The target profile is active, its signer locked or unlocked as the case needs, and synced.
