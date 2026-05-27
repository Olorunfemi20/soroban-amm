# Soroban AMM Python Client Examples

This folder contains Python scripts showing how to interact with the Soroban AMM contracts using the Stellar Python SDK (`stellar-sdk`).

It contains examples for the following contracts:
- **AMM Pool Contract** (`client.py`)
- **Factory Contract** (`factory_client.py`)
- **Governance Contract** (`governance_client.py`)

## Install

Set up a virtual environment and install the required dependencies:

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install -r requirements.txt
```

---

## 1. AMM Pool Client (`client.py`)

Demonstrates adding liquidity, quoting, and executing swaps on a specific pool.

### Configure

Set the environment variables before running the script:

```sh
export AMM_CONTRACT_ID=<deployed AMM contract id>
export SOURCE_SECRET=<secret key for the transaction source and LP/trader>
export TOKEN_IN_CONTRACT_ID=<token A or token B contract id>
export SWAP_AMOUNT_IN=100000
export SWAP_MIN_OUT=0
```

### Run

```sh
python client.py
```

---

## 2. Factory Client (`factory_client.py`)

Demonstrates deploying and querying pools registry via the pool factory.

### Configure

Set the environment variables before running the script:

```sh
export FACTORY_CONTRACT_ID=<deployed factory contract id>
export SOURCE_SECRET=<secret key for the transaction source and deployer>
export TOKEN_A_CONTRACT_ID=<token A contract id>
export TOKEN_B_CONTRACT_ID=<token B contract id>
export FEE_BPS=30 # Optional, defaults to 30
```

### Run

```sh
python factory_client.py
```

---

## 3. Governance Client (`governance_client.py`)

Demonstrates submitting fee proposals, querying proposal status/details, and casting LP-weighted votes (supporting `For`, `Against`, and `Abstain`).

### Configure

Set the environment variables before running the script:

```sh
export GOV_CONTRACT_ID=<deployed governance contract id>
export SOURCE_SECRET=<secret key for the proposer/voter LP holder>
export PROPOSAL_FEE_BPS=50 # Optional, target pool fee to propose (defaults to 50)
export PROPOSAL_ID=<proposal ID to query/vote on> # Optional, if not set, a new proposal is created
export VOTE_CHOICE=For # Optional: For, Against, or Abstain (defaults to For)
```

### Run

```sh
python governance_client.py
```
