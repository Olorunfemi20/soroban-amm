#!/usr/bin/env python3

import json
import os
import sys
from typing import Any, List, Optional

from stellar_sdk import Keypair, Network, scval
from stellar_sdk.address import Address
from stellar_sdk.contract import ContractClient

def main() -> int:
    rpc_url = os.getenv("STELLAR_RPC_URL", "https://soroban-testnet.stellar.org")
    network_passphrase = os.getenv(
        "STELLAR_NETWORK_PASSPHRASE",
        Network.TESTNET_NETWORK_PASSPHRASE,
    )

    factory_contract_id = required_env("FACTORY_CONTRACT_ID")
    source_secret = required_env("SOURCE_SECRET")
    token_a_contract_id = required_env("TOKEN_A_CONTRACT_ID")
    token_b_contract_id = required_env("TOKEN_B_CONTRACT_ID")
    fee_bps = int(os.getenv("FEE_BPS", "30"))

    source_keypair = Keypair.from_secret(source_secret)

    client = ContractClient(
        contract_id=factory_contract_id,
        rpc_url=rpc_url,
        network_passphrase=network_passphrase,
    )

    print(f"Connected to {rpc_url}")
    print(f"Factory contract: {factory_contract_id}")

    # 1. get_pool (read-only query)
    print("\n1. Querying if pool exists for pair...")
    pool_address = get_pool(client, token_a_contract_id, token_b_contract_id)
    print(f"Pool address: {pool_address}")

    # 2. create_pool if it doesn't exist
    if not pool_address:
        print(f"\n2. Creating pool for pair with fee_bps={fee_bps}...")
        try:
            pool_address = create_pool(client, source_keypair, token_a_contract_id, token_b_contract_id, fee_bps)
            print(f"Pool created successfully at: {pool_address}")
        except Exception as e:
            print(f"Failed to create pool (might already exist): {e}")
    else:
        print("\n2. Pool already exists, skipping create_pool.")

    # 3. all_pools
    print("\n3. Listing all pools deployed by this factory...")
    pools = all_pools(client)
    print(format_json(pools))

    client.server.close()
    return 0


def create_pool(client: ContractClient, source_kp: Keypair, token_a: str, token_b: str, fee_bps: int) -> str:
    result = submit_contract_call(
        client,
        source_kp,
        "create_pool",
        scval.to_address(token_a),
        scval.to_address(token_b),
        scval.to_int128(fee_bps),
    )
    if isinstance(result, Address):
        return result.address
    return str(result)


def get_pool(client: ContractClient, token_a: str, token_b: str) -> Optional[str]:
    result = simulate_contract_call(
        client,
        "get_pool",
        scval.to_address(token_a),
        scval.to_address(token_b),
    )
    if not result:
        return None
    if isinstance(result, Address):
        return result.address
    return str(result)


def all_pools(client: ContractClient) -> List[str]:
    result = simulate_contract_call(client, "all_pools")
    if not result:
        return []
    if isinstance(result, list):
        return [r.address if isinstance(r, Address) else str(r) for r in result]
    return [str(result)]


def simulate_contract_call(
    client: ContractClient,
    method: str,
    *parameters: Any,
) -> Any:
    return client.invoke(
        method,
        parameters=list(parameters),
        parse_result_xdr_fn=scval.to_native,
    ).result()


def submit_contract_call(
    client: ContractClient,
    source_keypair: Keypair,
    method: str,
    *parameters: Any,
) -> Any:
    assembled = client.invoke(
        method,
        parameters=list(parameters),
        source=source_keypair.public_key,
        signer=source_keypair,
        parse_result_xdr_fn=scval.to_native,
    )
    assembled.sign_auth_entries(source_keypair)
    return assembled.sign_and_submit()


def required_env(name: str) -> str:
    value = os.getenv(name)
    if not value:
        raise ValueError(f"Missing required environment variable: {name}")
    return value


def format_json(value: Any) -> str:
    return json.dumps(normalize_for_json(value), indent=2)


def normalize_for_json(value: Any) -> Any:
    if isinstance(value, Address):
        return value.address

    if isinstance(value, bytes):
        return value.hex()

    if isinstance(value, list):
        return [normalize_for_json(entry) for entry in value]

    if isinstance(value, dict):
        return {
            str(normalize_for_json(key)): normalize_for_json(entry_value)
            for key, entry_value in value.items()
        }

    return value


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(exc, file=sys.stderr)
        raise SystemExit(1) from exc
