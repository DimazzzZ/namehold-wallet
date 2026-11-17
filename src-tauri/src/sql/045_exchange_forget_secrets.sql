-- The Buyer's secret stays on disk only while the trade is followed
-- (exchange_io/store.rs, write_trade): forget it in trades that finished
-- before that rule. The version moves so a writer holding the old row
-- cannot write the secret back.
UPDATE exchange_trades
   SET holdings_json = json_set(holdings_json, '$.secret', json('null')),
       version = version + 1
 WHERE finished = 1
   AND holdings_json IS NOT NULL
   AND json_extract(holdings_json, '$.secret') IS NOT NULL;
