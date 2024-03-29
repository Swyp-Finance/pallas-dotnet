use std::collections::HashMap;

use lazy_static::lazy_static;
use pallas::{
    codec::utils::KeepRaw, ledger::{
        addresses::{Address, ByronAddress}, primitives::conway::{PlutusData, PseudoDatumOption}, traverse::MultiEraBlock
    }, network::{
        facades::NodeClient,
        miniprotocols::{
            chainsync::{self},
            Point as PallasPoint, MAINNET_MAGIC, PREVIEW_MAGIC, PRE_PRODUCTION_MAGIC,
            TESTNET_MAGIC, localstate::queries_v16,
        },
    }
};
use rnet::{net, Net};
use tokio::runtime::Runtime;

rnet::root!();

lazy_static! {
    static ref RT: Runtime = Runtime::new().expect("Failed to create Tokio runtime");
}

const DATUM_TYPE_HASH: u8 = 1;
const DATUM_TYPE_DATA: u8 = 2;

#[derive(Net)]
pub struct NetworkMagic {}

impl NetworkMagic {
    #[net]
    pub fn mainnet_magic() -> u64 {
        MAINNET_MAGIC
    }

    #[net]
    pub fn testnet_magic() -> u64 {
        TESTNET_MAGIC
    }

    #[net]
    pub fn preview_magic() -> u64 {
        PREVIEW_MAGIC
    }

    #[net]
    pub fn pre_production_magic() -> u64 {
        PRE_PRODUCTION_MAGIC
    }
}

#[derive(Net)]
pub struct Point {
    slot: u64,
    hash: Vec<u8>,
}

#[derive(Net)]
pub struct Block {
    slot: u64,
    hash: Vec<u8>,
    number: u64,
    transaction_bodies: Vec<TransactionBody>,
}

#[derive(Net)]
pub struct TransactionBody {
    id: Vec<u8>,
    inputs: Vec<TransactionInput>,
    outputs: Vec<TransactionOutput>,
    index: usize
}

#[derive(Net)]
pub struct TransactionInput {
    id: Vec<u8>,
    index: u64,
}


#[derive(Net)]
struct Datum {
    datum_type: u8,
    data: Option<Vec<u8>>,
}

#[derive(Net)]
pub struct TransactionOutput {
    address: Vec<u8>,
    amount: Value,
    index: usize,
    datum: Option<Datum>,
}

#[derive(Net)]
pub struct Value {
    coin: Coin,
    multi_asset: HashMap<PolicyId, HashMap<AssetName, Coin>>,
}

pub type Coin = u64;
pub type PolicyId = Vec<u8>;
pub type AssetName = Vec<u8>;

#[derive(Net)]
pub struct NextResponse {
    action: u8,
    tip: Option<Block>,
    block: Option<Block>,
}

#[derive(Net)]
pub struct NodeClientWrapper {
    client_ptr: usize,
}

fn convert_to_datum(datum: PseudoDatumOption<KeepRaw<'_, PlutusData>>) -> Datum {
    match datum {
        PseudoDatumOption::Hash(hash) => Datum {
            datum_type: DATUM_TYPE_HASH,
            data: Some(hash.to_vec()),
        },
        PseudoDatumOption::Data(keep_raw) => {
            let raw_data = keep_raw.raw_cbor().to_vec();
            Datum {
                datum_type: DATUM_TYPE_DATA,
                data: Some(raw_data),
            }
        },
    }
}

impl NodeClientWrapper {
    #[net]
    pub fn connect(socket_path: String, network_magic: u64) -> NodeClientWrapper {
        let client = RT.block_on(async {
            NodeClient::connect(&socket_path, network_magic)
                .await
                .unwrap()
        });

        let client_box = Box::new(client);
        let client_ptr = Box::into_raw(client_box) as usize;

        NodeClientWrapper { client_ptr }
    }

    #[net]
    pub fn get_tip(client_wrapper: NodeClientWrapper) -> Point {
        unsafe {
            let client_ptr = client_wrapper.client_ptr as *mut NodeClient;

            // Convert the raw pointer back to a Box to deallocate the memory
            let mut client = Box::from_raw(client_ptr);

            // Get the tip
            let tip = RT.block_on(async {
                let state_query_client = client.statequery();

                state_query_client.acquire(None).await.unwrap();

                queries_v16::get_chain_point(state_query_client).await.unwrap()
            });

            // Convert client back to a raw pointer for future use
            let _ = Box::into_raw(client);

            match tip {
                PallasPoint::Origin => Point {
                    slot: 0,
                    hash: vec![],
                },
                PallasPoint::Specific(slot, hash) => Point { slot, hash },
            }
        }
    }

    #[net]
    pub fn find_intersect(client_wrapper: NodeClientWrapper, known_point: Point) -> Option<Point> {
        unsafe {
            let client_ptr = client_wrapper.client_ptr as *mut NodeClient;

            // Convert the raw pointer back to a Box to deallocate the memory
            let mut _client = Box::from_raw(client_ptr);
            let client = _client.chainsync();

            let known_points = vec![PallasPoint::Specific(known_point.slot, known_point.hash)];

            // Get the intersecting point and the tip
            let (intersect_point, _tip) =
                RT.block_on(async { client.find_intersect(known_points).await.unwrap() });

            // Convert client back to a raw pointer for future use
            let _ = Box::into_raw(_client);

            // Match on the intersecting point
            intersect_point.map(|pallas_point| match pallas_point {
                PallasPoint::Origin => Point {
                    slot: 0,
                    hash: vec![],
                },
                PallasPoint::Specific(slot, hash) => Point { slot, hash },
            })
        }
    }

    #[net]
    pub fn chain_sync_next(client_wrapper: NodeClientWrapper) -> NextResponse {
        unsafe {
            let client_ptr = client_wrapper.client_ptr as *mut NodeClient;

            // Convert the raw pointer back to a Box to deallocate the memory
            let mut client = Box::from_raw(client_ptr);

            // Get the next block
            let result = RT.block_on(async {
                if client.chainsync().has_agency() {
                    // When the client has the agency, send a request for the next block
                    client.chainsync().request_next().await
                } else {
                    // When the client does not have the agency, wait for the server's response
                    client.chainsync().recv_while_must_reply().await
                }
            });

            let next_response = match result {
                Ok(next) => match next {
                    chainsync::NextResponse::RollForward(h, tip) => match MultiEraBlock::decode(&h)
                    {
                        Ok(b) => NextResponse {
                            action: 1,
                            tip: match tip.0 {
                                PallasPoint::Origin => Some(Block {
                                    slot: 0,
                                    hash: vec![],
                                    number: 0,
                                    transaction_bodies: vec![],
                                }),
                                PallasPoint::Specific(slot, hash) => Some(Block {
                                    slot,
                                    hash,
                                    number: tip.1,
                                    transaction_bodies: vec![],
                                }),
                            },
                            block: Some(Block {
                                slot: b.slot(),
                                hash: b.hash().to_vec(),
                                number: b.number(),
                                transaction_bodies: b
                                    .txs()
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, tx_body)| TransactionBody {
                                        id: tx_body.hash().to_vec(),
                                        index,
                                        inputs: tx_body
                                            .inputs()
                                            .into_iter()
                                            .map(|tx_input| TransactionInput {
                                                id: tx_input.hash().to_vec(),
                                                index: tx_input.index(),
                                            })
                                            .collect(),
                                        outputs: tx_body
                                            .outputs()
                                            .into_iter()
                                            .enumerate()
                                            .map(|(index, tx_output)| TransactionOutput {
                                                index,
                                                address: tx_output.address().unwrap().to_vec(),
                                                datum: tx_output
                                                    .datum()
                                                    .map(convert_to_datum),
                                                amount: Value {
                                                    coin: tx_output.lovelace_amount(),
                                                    multi_asset: tx_output
                                                        .non_ada_assets()
                                                        .iter()
                                                        .filter(|ma| ma.is_output())
                                                        .map(|ma| {
                                                            (
                                                                ma.policy().to_vec(),
                                                                ma.assets()
                                                                    .iter()
                                                                    .map(|a| {
                                                                        (
                                                                            a.name().to_vec(),
                                                                            a.output_coin()
                                                                                .unwrap(),
                                                                        )
                                                                    })
                                                                    .collect(),
                                                            )
                                                        })
                                                        .collect(),
                                                },
                                            })
                                            .collect(),
                                    })
                                    .collect(),
                            }),
                        },
                        Err(e) => {
                            println!("error: {:?}", e);
                            NextResponse {
                                action: 0,
                                block: None,
                                tip: None,
                            }
                        }
                    },
                    chainsync::NextResponse::RollBackward(point, tip) => NextResponse {
                        action: 2,
                        tip: match tip.0 {
                            PallasPoint::Origin => Some(Block {
                                slot: 0,
                                hash: vec![],
                                number: 0,
                                transaction_bodies: vec![],
                            }),
                            PallasPoint::Specific(slot, hash) => Some(Block {
                                slot,
                                hash,
                                number: tip.1,
                                transaction_bodies: vec![],
                            }),
                        },
                        block: match point {
                            PallasPoint::Origin => Some(Block {
                                slot: 0,
                                hash: vec![],
                                number: 0,
                                transaction_bodies: vec![],
                            }),
                            PallasPoint::Specific(slot, hash) => Some(Block {
                                slot,
                                hash,
                                number: 0,
                                transaction_bodies: vec![],
                            }),
                        },
                    },
                    chainsync::NextResponse::Await => NextResponse {
                        action: 3,
                        tip: None,
                        block: None,
                    },
                },
                Err(e) => {
                    println!("chain_sync_next error: {:?}", e);
                    NextResponse {
                        action: 0,
                        block: None,
                        tip: None,
                    }
                }
            };

            // Convert client back to a raw pointer for future use
            let _ = Box::into_raw(client);
            next_response
        }
    }

    #[net]
    pub fn chain_sync_has_agency(client_wrapper: NodeClientWrapper) -> bool {
        unsafe {
            let client_ptr = client_wrapper.client_ptr as *mut NodeClient;

            // Convert the raw pointer back to a Box to deallocate the memory
            let mut _client = Box::from_raw(client_ptr);

            let has_agency = _client.chainsync().has_agency();

            // Convert client back to a raw pointer for future use
            let _ = Box::into_raw(_client);

            has_agency
        }
    }

    #[net]
    pub fn address_bytes_to_bech32(address_bytes: Vec<u8>) -> String {
        match Address::from_bytes(&address_bytes).unwrap().to_bech32() {
            Ok(address) => address,
            Err(_) => ByronAddress::from_bytes(&address_bytes)
                .unwrap()
                .to_base58(),
        }
    }

    #[net]
    pub fn disconnect(client_wrapper: NodeClientWrapper) {
        unsafe {
            // Convert the usize back to a raw pointer
            let client_ptr = client_wrapper.client_ptr as *mut NodeClient;

            // Convert the raw pointer back to a Box to deallocate the memory
            let mut _client = Box::from_raw(client_ptr);

            _client.abort();
            // Memory is deallocated when _client goes out of scope
        }
    }
}
