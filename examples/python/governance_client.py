#!/usr/bin/env python3

import json
import os
import sys
from typing import Any, Dict

from stellar_sdk import Keypair, Network, scval
from stellar_sdk.address import Address
from stellar_sdk.contract import ContractClient

def main() -> int:
    rpc_url = os.getenv("STELLAR_RPC_URL", "https://soroban-testnet.stellar.org")
    network_passphrase = os.getenv(
        "STELLAR_NETWORK_PASSPHRASE",
        Network.TESTNET_NETWORK_PASSPHRASE,
    )

    gov_contract_id = required_env("GOV_CONTRACT_ID")
    source_secret = required_env("SOURCE_SECRET")
    proposal_fee_bps = int(os.getenv("PROPOSAL_FEE_BPS", "50"))
    proposal_id = os.getenv("PROPOSAL_ID")

    source_keypair = Keypair.from_secret(source_secret)

    client = ContractClient(
        contract_id=gov_contract_id,
        rpc_url=rpc_url,
        network_passphrase=network_passphrase,
    )

    print(f"Connected to {rpc_url}")
    print(f"Governance contract: {gov_contract_id}")

    # 1. Propose fee change
    if not proposal_id:
        print(f"\n1. Submitting new proposal to change fee to {proposal_fee_bps} bps...")
        try:
            pid = propose(client, source_keypair, proposal_fee_bps)
            print(f"Proposal created with ID: {pid}")
            proposal_id = pid
        except Exception as e:
            print(f"Failed to submit proposal (make sure proposer has enough LP stake): {e}")
            return 1
    else:
        proposal_id = int(proposal_id)
        print(f"\n1. Using existing proposal ID: {proposal_id}")

    # 2. Query proposal status
    print(f"\n2. Reading status of proposal {proposal_id}...")
    status = proposal_status(client, proposal_id)
    print(f"Proposal status: {status}")

    # 3. Query proposal details
    print(f"\n3. Reading details of proposal {proposal_id}...")
    proposal_info = get_proposal(client, proposal_id)
    print(format_json(proposal_info))

    # 4. Vote on the proposal (if active)
    if status == "Active" or (isinstance(status, dict) and "Active" in status) or status == {"Active": []}:
        vote_choice = os.getenv("VOTE_CHOICE", "For")  # For, Against, Abstain
        print(f"\n4. Voting '{vote_choice}' on proposal {proposal_id}...")
        try:
            vote(client, source_keypair, proposal_id, vote_choice)
            print("Vote cast successfully!")
        except Exception as e:
            print(f"Failed to cast vote: {e}")

    client.server.close()
    return 0


def propose(client: ContractClient, proposer_kp: Keypair, new_fee_bps: int) -> int:
    # ProposalKind::UpdateFee(new_fee_bps) is encoded as a vector of [Symbol("UpdateFee"), i128]
    kind_scval = scval.to_vec([
        scval.to_symbol("UpdateFee"),
        scval.to_int128(new_fee_bps)
    ])
    result = submit_contract_call(
        client,
        proposer_kp,
        "propose",
        scval.to_address(proposer_kp.public_key),
        kind_scval,
    )
    return int(result)


def vote(client: ContractClient, voter_kp: Keypair, proposal_id: int, choice: str) -> None:
    # choice must be For, Against, or Abstain
    if choice not in ("For", "Against", "Abstain"):
        raise ValueError(f"Invalid vote choice '{choice}'. Must be For, Against, or Abstain.")
    
    choice_scval = scval.to_symbol(choice)
    submit_contract_call(
        client,
        voter_kp,
        "vote",
        scval.to_address(voter_kp.public_key),
        scval.to_uint32(proposal_id),
        choice_scval,
    )


def get_proposal(client: ContractClient, proposal_id: int) -> Dict[str, Any]:
    result = simulate_contract_call(
        client,
        "get_proposal",
        scval.to_uint32(proposal_id),
    )
    return result


def proposal_status(client: ContractClient, proposal_id: int) -> str:
    result = simulate_contract_call(
        client,
        "proposal_status",
        scval.to_uint32(proposal_id),
    )
    if isinstance(result, dict):
        # Enums are parsed as dictionaries like {"Active": []} or "Active"
        return next(iter(result.keys()))
    return str(result)


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
