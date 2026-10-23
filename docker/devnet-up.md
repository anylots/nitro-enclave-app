下面是基于当前仓库、全新本地 Anvil L1 的完整 contract-mode devnet 启动流程。当前首次启动不要直接运行 `make devnet-up`，因为 `scheduleEpoch` 还需要在 deploy 和 up 之间手动执行。

## 一、准备依赖

在仓库根目录：

```bash
git submodule update --init sotto-reth
yarn --cwd contracts install --frozen-lockfile

forge --version
cast --version
node --version
docker compose version
```

当前 genesis 生成脚本需要 Node.js。

## 二、启动全新的 L1 Anvil

单独开一个终端，保持运行：

```bash
anvil \
  --host 0.0.0.0 \
  --port 9545 \
  --chain-id 2910
```

不要在部署过程中用其他程序发送 account 0 的交易，否则预期合约地址会变化。

另一个终端进入仓库根目录，设置环境变量：

```bash
export L1_RPC_URL=http://127.0.0.1:9545
export ETH_RPC_URL="$L1_RPC_URL"

export DEPLOYER_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

export VERIFIER_CONFIG="$PWD/ops/verifier/verifier-config.local.json"
export VERIFIER_OUTPUT_DIR="$PWD/ops/verifier/.deployment"

export DEPLOY_CONFIG="$PWD/ops/devnet/deploy-config.local.json"
export DEPLOY_OUTPUT_DIR="$PWD/ops/devnet/.deployment"
export DEPLOY_GENESIS="$DEPLOY_OUTPUT_DIR/l2-genesis.json"
```

确认 L1：

```bash
cast chain-id --rpc-url "$L1_RPC_URL"
```

应返回：

```text
2910
```

全新部署要求下面两个输出目录为空：

```text
ops/verifier/.deployment
ops/devnet/.deployment
```

如果里面是旧网络记录，应先归档到其他目录，不要把旧部署记录复用于一个已经重置的 Anvil。

## 三、部署 verifier 基础合约

```bash
make verifier-deploy
```

当前本地配置按顺序应部署出：

```text
SP1Verifier:       0x5FbDB2315678afecb367f032d93F642f64180aa3
TEEProverRegistry: 0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512
TEEVerifier:       0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0
```

可以复核：

```bash
make verifier-verify
```

## 四、部署本地 L1 BGB

Verifier 部署使用 account 0 的 nonce 0–2，BGB 应使用 nonce 3，从而得到配置中的地址 `0xCf7E...Fc9`。

```bash
(
  cd contracts
  forge create \
    contracts/test/AssetPathRevision.t.sol:AssetRevisionToken \
    --constructor-args "Bitget Token" "BGB" 18 \
    --rpc-url "$L1_RPC_URL" \
    --private-key "$DEPLOYER_PRIVATE_KEY" \
    --broadcast
)
```

验证：

```bash
cast call \
  0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9 \
  'decimals()(uint8)' \
  --rpc-url "$L1_RPC_URL"

cast nonce \
  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
  --rpc-url "$L1_RPC_URL"
```

预期：

```text
18
4
```

nonce 必须是 `4`，因为下一个部署的 `L1DeploymentFactory` 配置地址 `0xDc64...F6C9` 是 account 0 nonce 4 对应的 CREATE 地址。

## 五、生成部署计划和 L2 genesis

生成 L1 地址、部署操作、L2 system alloc 和 governance actions：

```bash
make devnet-prepare
```

生成完整 L2 genesis，并通过临时 Anvil验证 block hash/state root：

```bash
make devnet-genesis
```

输出文件：

```text
ops/devnet/.deployment/l2-genesis.json
```

实际启动 contract-mode L2 时使用的是这个文件，不是直接使用 `ops/devnet/genesis.json` 的 fixture alloc。

## 六、部署 L1 网络合约

```bash
make devnet-deploy
```

这一步会：

- 部署 DeploymentFactory
- 部署 ProxyAdmin
- 部署 9 个 L1 implementation
- 部署并初始化 9 个 proxy
- 自动执行 `registerVerifier`
- 自动执行 `setPcr0Trusted`
- 检查完整 L2 genesis
- 记录交易和 receipt
- 将 phase 设置为 `l1-contracts-verified`

检查状态：

```bash
node -e '
const s = require("./ops/devnet/.deployment/deployment-state.json");
console.log(s.phase);
console.log(s.transactions.map(x => x.name));
'
```

## 七、执行 `scheduleEpoch`

目前它仍是手动治理操作，必须在启动 settler 前执行：

```bash
cast send \
  0x9e82784b1838abF2049c5A01536dbA1066B9b7EA \
  'scheduleEpoch(uint64,(uint64,uint64,bytes32))' \
  1 \
  '(1,100000,0x66eb2c793e6d6578ee3452de98c14744c925d2f5fc8ded00ac87812942d3d6a3)' \
  --rpc-url "$L1_RPC_URL" \
  --private-key "$DEPLOYER_PRIVATE_KEY"
```

验证：

```bash
cast call \
  0x9e82784b1838abF2049c5A01536dbA1066B9b7EA \
  'getEpoch(uint64)((uint64,uint64,bytes32))' \
  1 \
  --rpc-url "$L1_RPC_URL"
```

预期：

```text
(1, 100000, 0x66eb2c...)
```

在 settler 尚未提交第一个 batch 前，刷新部署记录：

```bash
make devnet-verify
```

此时 `governance-actions.json` 仍可能保留一个 L2 fee recipient approval，这是正常的。它不阻塞出块和 batch settlement。

如果已经误启动了 settler，应先停止它，避免 `scheduleEpoch` 成功后立即提交 batch，导致初始部署验证无法再运行：

```bash
docker stop settlement-settler
make devnet-verify
```

## 八、启动完整 L2 网络

让容器内的 deposit watcher 和 settler 能访问宿主机 Anvil：

```bash
export SOTTO_DEVNET_L1_RPC_URL=http://host.docker.internal:9545
unset DEPLOYER_PRIVATE_KEY

DEVNET_MODE=contracts make devnet-up
```

第一次执行会构建：

- 4 个 sotto-reth
- 3 个 raft sequencer 节点
- 1 个 P2P fullnode
- settlement settler

完成时脚本会检查：

- leader 已选出
- 4 个 reth 高度均大于 0
- L2TokenRegistry RPC 查询成功

如果之前手动停止过 settler：

```bash
docker start settlement-settler
```

## 九、启动后检查

容器状态：

```bash
docker compose \
  --env-file ops/devnet/.generated/.env \
  -f ops/devnet/docker-compose.yml \
  ps
```

Leader：

```bash
./ops/devnet/run.sh leader
```

四个 reth 高度：

```bash
./ops/devnet/run.sh height
```

L2 chain ID：

```bash
cast chain-id --rpc-url http://127.0.0.1:8545
```

应返回：

```text
53077
```

关键日志：

```bash
docker logs --tail 100 sotto-node-0
docker logs --tail 100 reth-node-0
docker logs --tail 100 settlement-settler
```

Settler 日志中不应再出现：

```text
L1SequencerRegistry: unknown epoch
```

查看 Rollup batch 进度：

```bash
cast call \
  0x321c0449005CDF8546738a74C9E7C40784fb83bF \
  'nextBatchIndex()(uint64)' \
  --rpc-url "$L1_RPC_URL"
```

## 十、日常停止和重启

停止容器但保留 L2 数据：

```bash
make devnet-down
```

重新启动同一个网络：

```bash
export L1_RPC_URL=http://127.0.0.1:9545
export SOTTO_DEVNET_L1_RPC_URL=http://host.docker.internal:9545

DEVNET_MODE=contracts make devnet-up
```

前提是 L1 Anvil 仍然保留原来的链上状态。若 Anvil 已重置为空链，不能复用旧 `.deployment`。

## 十一、修改 settlement 代码后单独重建

```bash
docker compose \
  --env-file ops/devnet/.generated/.env \
  -f ops/devnet/docker-compose.yml \
  up -d --build --no-deps settlement-settler
```

然后：

```bash
docker logs -f --tail 100 settlement-settler
```

单纯执行 `docker restart settlement-settler` 不会包含新代码。

## 十二、完全重建网络

`make devnet-clean` 会删除：

- Docker 容器和 volumes
- `ops/devnet/.generated`
- JWT
- L2 节点数据

但会保留：

```text
ops/devnet/.deployment
```

如果 L1 Anvil 也已经重置，需要同时使用全新的、空的：

```text
ops/verifier/.deployment
ops/devnet/.deployment
```

然后从 verifier 部署重新开始。已经向 L1 提交过 batch 的网络，不应只清理 L2 后从 genesis 重启。