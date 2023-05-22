use std::collections::HashMap;

use rust_decimal::prelude::*;
use rust_decimal::Decimal;

fn main() {
    println!("Hello, Coreum!");

    let _ = calculate_balance_changes(
        vec![],
        vec![],
        MultiSend {
            inputs: vec![],
            outputs: vec![],
        },
    );
}

// A user can submit a `MultiSend` transaction (similar to bank.MultiSend in cosmos sdk) to transfer multiple
// coins (denoms) from multiple input addresses to multiple output addresses. A denom is the name or symbol
// for a coin type, e.g USDT and USDC can be considered different denoms; in cosmos ecosystem they are called
// denoms, in ethereum world they are called symbols.
// The sum of input coins and output coins must match for every transaction.
struct MultiSend {
    // inputs contain the list of accounts that want to send coins from, and how many coins from each account we want to send.
    inputs: Vec<Balance>,
    // outputs contains the list of accounts that we want to deposit coins into, and how many coins to deposit into
    // each account
    outputs: Vec<Balance>,
}

impl MultiSend {
    fn validate_inout(&self, definition: &DenomDefinition) -> Result<(), String> {
        let input_sum = self.inputs.get_coin_sum(&definition.denom);
        let output_sum = self.outputs.get_coin_sum(&definition.denom);

        if input_sum != output_sum {
            return Err("Input and output mismatch".to_string());
        }

        Ok(())
    }

    fn process_input(
        &self,
        definition: &DenomDefinition,
        changes: &mut HashMap<(String, String), i128>,
    ) -> Result<(), String> {
        let non_issuer_input_sum = self.inputs.get_filtered_coin_sum(definition);
        let non_issuer_output_sum = self.outputs.get_filtered_coin_sum(definition);

        let (denominate, numerate) = if non_issuer_input_sum > non_issuer_output_sum {
            (non_issuer_input_sum, non_issuer_output_sum)
        } else {
            (non_issuer_output_sum, non_issuer_input_sum)
        };

        let burn_rate =
            Decimal::from_f64(definition.burn_rate).ok_or("Decimal issue".to_string())?;
        let commission_rate =
            Decimal::from_f64(definition.commission_rate).ok_or("Decimal issue".to_string())?;

        for input in &self.inputs {
            if let Some(coin) = input.coins.find_coin(&definition.denom) {
                let amount = Decimal::from_i128(coin.amount).ok_or("Decimal issue".to_string())?;
                let mut burnt = amount.saturating_mul(burn_rate);
                let mut commission = amount.saturating_mul(commission_rate);

                if denominate != numerate {
                    let numerate =
                        Decimal::from_i128(numerate).ok_or("Decimal issue".to_string())?;
                    let denominate =
                        Decimal::from_i128(denominate).ok_or("Decimal issue".to_string())?;

                    burnt = burnt
                        .saturating_mul(numerate)
                        .checked_div(denominate)
                        .ok_or("Calculation failure".to_string())?;

                    commission = commission
                        .saturating_mul(numerate)
                        .checked_div(denominate)
                        .ok_or("Calculation failure".to_string())?;
                };

                let input_key = (input.address.clone(), definition.denom.clone());
                let output_key = (definition.issuer.clone(), definition.denom.clone());

                let burnt = burnt.ceil().to_i128().ok_or("Decimal issue".to_string())?;
                let commission = commission
                    .ceil()
                    .to_i128()
                    .ok_or("Decimal issue".to_string())?;

                *changes.entry(input_key.clone()).or_insert(0) -= burnt + commission + coin.amount;
                *changes.entry(output_key.clone()).or_insert(0) += commission;
            }
        }

        Ok(())
    }

    fn process_output(
        &self,
        definition: &DenomDefinition,
        changes: &mut HashMap<(String, String), i128>,
    ) {
        for output in &self.outputs {
            if let Some(coin) = output.coins.find_coin(&definition.denom) {
                *changes
                    .entry((output.address.clone(), coin.denom.clone()))
                    .or_insert(0) += coin.amount;
            }
        }
    }

    fn process(
        &self,
        definitions: &[DenomDefinition],
        changes: &mut HashMap<(String, String), i128>,
    ) -> Result<(), String> {
        for definition in definitions {
            self.validate_inout(definition)?;
            self.process_input(definition, changes)?;
            self.process_output(definition, changes);
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct Coin {
    pub denom: String,
    pub amount: i128,
}

trait CoinOp {
    fn find_coin(&self, denom: &str) -> Option<&Coin>;
}

impl CoinOp for Vec<Coin> {
    fn find_coin(&self, denom: &str) -> Option<&Coin> {
        self.iter().find(|coin| coin.denom == denom)
    }
}

#[derive(Debug)]
struct Balance {
    address: String,
    coins: Vec<Coin>,
}

impl CoinOp for Balance {
    fn find_coin(&self, denom: &str) -> Option<&Coin> {
        self.coins.find_coin(denom)
    }
}

trait BalanceOp {
    fn get_coin_sum(&self, denom: &str) -> i128;

    fn get_filtered_coin_sum(&self, skip_denom: &DenomDefinition) -> i128;
}

impl BalanceOp for Vec<Balance> {
    fn get_coin_sum(&self, denom: &str) -> i128 {
        self.iter()
            .filter_map(|balance| balance.find_coin(denom))
            .map(|coin| coin.amount)
            .sum()
    }

    fn get_filtered_coin_sum(&self, skip_denom: &DenomDefinition) -> i128 {
        self.iter()
            .filter_map(|balance| {
                if skip_denom.issuer == balance.address {
                    None
                } else {
                    balance.find_coin(&skip_denom.denom)
                }
            })
            .map(|coin| coin.amount)
            .sum()
    }
}

// A Denom has a definition (`CoinDefinition`) which contains different attributes related to the denom:
struct DenomDefinition {
    // the unique identifier for the token (e.g `core`, `eth`, `usdt`, etc.)
    denom: String,
    // The address that created the token
    issuer: String,
    // burn_rate is a number between 0 and 1. If it is above zero, in every transfer,
    // some additional tokens will be burnt on top of the transferred value, from the senders address.
    // The tokens to be burnt are calculated by multiplying the TransferAmount by burn rate, and
    // rounding it up to an integer value. For example if an account sends 100 token and burn_rate is
    // 0.2, then 120 (100 + 100 * 0.2) will be deducted from sender account and 100 will be deposited to the recipient
    // account (i.e 20 tokens will be burnt)
    burn_rate: f64,
    // commission_rate is exactly same as the burn_rate, but the calculated value will be transferred to the
    // issuer's account address instead of being burnt.
    commission_rate: f64,
}

fn to_hashmap(vec: &[Balance]) -> HashMap<(String, String), i128> {
    vec.iter()
        .flat_map(|balance| {
            balance
                .coins
                .iter()
                .map(move |coin| ((balance.address.clone(), coin.denom.clone()), coin.amount))
        })
        .collect()
}

fn from_hashmap(map: &HashMap<(String, String), i128>) -> Vec<Balance> {
    let mut balances: HashMap<String, Balance> = HashMap::new();

    for ((address, denom), &amount) in map {
        if amount == 0 {
            continue;
        }

        let balance = balances.entry(address.clone()).or_insert_with(|| Balance {
            address: address.clone(),
            coins: Vec::new(),
        });

        balance.coins.push(Coin {
            denom: denom.clone(),
            amount,
        });
    }

    balances.into_values().collect()
}

// Implement `calculate_balance_changes` with the following requirements.
// - Output of the function is the balance changes that must be applied to different accounts
//   (negative means deduction, positive means addition), or an error. the error indicates that the transaction must be rejected.
// - If sum of inputs and outputs in multi_send_tx does not match the tx must be rejected(i.e return error).
// - Apply burn_rate and commission_rate as described by their definition.
// - If the sender does not have enough balances (in the original_balances) to cover the input amount on top of burn_rate and
// commission_rate, the transaction must be rejected.
// - burn_rate and commission_rate does not apply to the issuer. So to calculate the correct values you must do this for every denom:
//      - sum all the inputs coming from accounts that are not an issuer (let's call it non_issuer_input_sum)
//      - sum all the outputs going to accounts that are not an issuer (let's call it non_issuer_output_sum)
//      - total burn amount is total_burn = min(non_issuer_input_sum, non_issuer_output_sum)
//      - total_burn is distributed between all input accounts as: account_share = roundup(total_burn * input_from_account / non_issuer_input_sum)
//      - total_burn_amount = sum (account_shares) // notice that in previous step we rounded up, so we need to recalculate the total again.
//      - commission_rate is exactly the same, but we send the calculate value to issuer, and not burn.
//      - Example:
//          burn_rate: 10%
//
//          inputs:
//          60, 90
//          25 <-- issuer
//
//          outputs:
//          50
//          100 <-- issuer
//          25
//          In this case burn amount is: min(non_issuer_inputs, non_issuer_outputs) = min(75+75, 50+25) = 75
//          Expected burn: 75 * 10% = 7.5
//          And now we divide it proportionally between all input sender: first_sender_share  = 7.5 * 60 / 150  = 3
//                                                                        second_sender_share = 7.5 * 90 / 150  = 4.5
// - In README.md we have provided more examples to help you better understand the requirements.
// - Write different unit tests to cover all the edge cases, we would like to see how you structure your tests.
//   There are examples in README.md, you can convert them into tests, but you should add more cases.
fn calculate_balance_changes(
    original_balances: Vec<Balance>,
    definitions: Vec<DenomDefinition>,
    multi_send_tx: MultiSend,
) -> Result<Vec<Balance>, String> {
    let mut balances_changes_map: HashMap<(String, String), i128> = HashMap::new();
    multi_send_tx.process(&definitions, &mut balances_changes_map)?;
    let original_balances_map = to_hashmap(&original_balances);

    for (key, &amount) in balances_changes_map.iter() {
        if amount >= 0 {
            continue;
        }

        let origin = original_balances_map.get(key);
        if origin.is_none() || origin.unwrap() < &(-amount) {
            return Err("Insufficient balance".to_string());
        }
    }

    let balances_changes = from_hashmap(&balances_changes_map);

    Ok(balances_changes)
}

#[test]
fn test_no_issuer_on_sender_or_receiver() {
    let original_balances = vec![
        Balance {
            address: "account1".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 1000000,
            }],
        },
        Balance {
            address: "account2".to_string(),
            coins: vec![Coin {
                denom: "denom2".to_string(),
                amount: 1000000,
            }],
        },
    ];
    let definitions = vec![
        DenomDefinition {
            denom: "denom1".to_string(),
            issuer: "issuer_account_A".to_string(),
            burn_rate: 0.08,
            commission_rate: 0.12,
        },
        DenomDefinition {
            denom: "denom2".to_string(),
            issuer: "issuer_account_B".to_string(),
            burn_rate: 1.,
            commission_rate: 0.,
        },
    ];
    let multi_send_tx = MultiSend {
        inputs: vec![
            Balance {
                address: "account1".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 1000,
                }],
            },
            Balance {
                address: "account2".to_string(),
                coins: vec![Coin {
                    denom: "denom2".to_string(),
                    amount: 1000,
                }],
            },
        ],
        outputs: vec![Balance {
            address: "account_recipient".to_string(),
            coins: vec![
                Coin {
                    denom: "denom1".to_string(),
                    amount: 1000,
                },
                Coin {
                    denom: "denom2".to_string(),
                    amount: 1000,
                },
            ],
        }],
    };

    let res = calculate_balance_changes(original_balances, definitions, multi_send_tx).unwrap();

    let account1 = res.iter().find(|e| e.address == "account1").unwrap();

    assert_eq!(account1.coins.len(), 1);

    let account1_denom1 = account1.coins.iter().find(|e| e.denom == "denom1").unwrap();
    assert_eq!(account1_denom1.amount, -1200);
}

#[test]
fn test_issuer_on_sender_or_receiver() {
    let original_balances = vec![
        Balance {
            address: "account1".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 1000000,
            }],
        },
        Balance {
            address: "account2".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 1000000,
            }],
        },
    ];
    let definitions = vec![DenomDefinition {
        denom: "denom1".to_string(),
        issuer: "issuer_account_A".to_string(),
        burn_rate: 0.08,
        commission_rate: 0.12,
    }];
    let multi_send_tx = MultiSend {
        inputs: vec![
            Balance {
                address: "account1".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 650,
                }],
            },
            Balance {
                address: "account2".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 350,
                }],
            },
        ],
        outputs: vec![
            Balance {
                address: "account_recipient".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 500,
                }],
            },
            Balance {
                address: "issuer_account_A".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 500,
                }],
            },
        ],
    };

    let res = calculate_balance_changes(original_balances, definitions, multi_send_tx).unwrap();

    let account1 = res.iter().find(|e| e.address == "account1").unwrap();

    assert_eq!(account1.coins.len(), 1);

    let account1_denom1 = account1.coins.iter().find(|e| e.denom == "denom1").unwrap();
    assert_eq!(account1_denom1.amount, -715);
}

#[test]
fn test_not_enough_balance() {
    let original_balances = vec![Balance {
        address: "account1".to_string(),
        coins: vec![],
    }];
    let definitions = vec![DenomDefinition {
        denom: "denom1".to_string(),
        issuer: "issuer_account_A".to_string(),
        burn_rate: 0.,
        commission_rate: 0.,
    }];
    let multi_send_tx = MultiSend {
        inputs: vec![Balance {
            address: "account1".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 350,
            }],
        }],
        outputs: vec![Balance {
            address: "account_recipient".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 350,
            }],
        }],
    };

    let res = calculate_balance_changes(original_balances, definitions, multi_send_tx);

    match res {
        Err(value) => assert_eq!(value, "Insufficient balance".to_string()),
        Ok(_) => panic!("wrong"),
    }
}

#[test]
fn test_input_output_mismatch() {
    let original_balances = vec![Balance {
        address: "account1".to_string(),
        coins: vec![Coin {
            denom: "denom1".to_string(),
            amount: 1000000,
        }],
    }];
    let definitions = vec![DenomDefinition {
        denom: "denom1".to_string(),
        issuer: "issuer_account_A".to_string(),
        burn_rate: 0.,
        commission_rate: 0.,
    }];
    let multi_send_tx = MultiSend {
        inputs: vec![Balance {
            address: "account1".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 350,
            }],
        }],
        outputs: vec![Balance {
            address: "account_recipient".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 450,
            }],
        }],
    };

    let res = calculate_balance_changes(original_balances, definitions, multi_send_tx);

    match res {
        Err(value) => assert_eq!(value, "Input and output mismatch".to_string()),
        Ok(_) => panic!("wrong"),
    }
}

#[test]
fn test_rounding_up() {
    let original_balances = vec![
        Balance {
            address: "account1".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 1000,
            }],
        },
        Balance {
            address: "account2".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 1000,
            }],
        },
    ];
    let definitions = vec![DenomDefinition {
        denom: "denom1".to_string(),
        issuer: "issuer_account_A".to_string(),
        burn_rate: 0.01,
        commission_rate: 0.01,
    }];
    let multi_send_tx = MultiSend {
        inputs: vec![
            Balance {
                address: "account1".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 1,
                }],
            },
            Balance {
                address: "account2".to_string(),
                coins: vec![Coin {
                    denom: "denom1".to_string(),
                    amount: 1,
                }],
            },
        ],
        outputs: vec![Balance {
            address: "account_recipient".to_string(),
            coins: vec![Coin {
                denom: "denom1".to_string(),
                amount: 2,
            }],
        }],
    };

    let res = calculate_balance_changes(original_balances, definitions, multi_send_tx).unwrap();

    let account1 = res.iter().find(|e| e.address == "account1").unwrap();

    assert!(account1.coins[0].amount == -3);

    let account2 = res.iter().find(|e| e.address == "account2").unwrap();

    assert!(account2.coins[0].amount == -3);

    let account_recipient = res
        .iter()
        .find(|e| e.address == "account_recipient")
        .unwrap();

    assert!(account_recipient.coins[0].amount == 2);

    let issuer_account_a = res
        .iter()
        .find(|e| e.address == "issuer_account_A")
        .unwrap();

    assert!(issuer_account_a.coins[0].amount == 2);
}
