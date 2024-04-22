package chainio

import (
	"github.com/Layr-Labs/eigensdk-go/chainio/clients/eth"
	"github.com/Layr-Labs/eigensdk-go/logging"

	gethcommon "github.com/ethereum/go-ethereum/common"

	csservicemanager "github.com/yetanotherco/aligned_layer/contracts/bindings/AlignedLayerServiceManager"
)

type AvsServiceBindings struct {
	ServiceManager *csservicemanager.ContractAlignedLayerServiceManager
	ethClient      eth.Client
	logger         logging.Logger
}

func NewAvsServiceBindings(serviceManagerAddr, blsOperatorStateRetrieverAddr gethcommon.Address, ethclient eth.Client, logger logging.Logger) (*AvsServiceBindings, error) {
	contractServiceManager, err := csservicemanager.NewContractAlignedLayerServiceManager(serviceManagerAddr, ethclient)
	if err != nil {
		logger.Error("Failed to fetch IServiceManager contract", "err", err)
		return nil, err
	}

	// taskManagerAddr, err := contractServiceManager.AlignedLayerTaskManager(&bind.CallOpts{})
	// if err != nil {
	// 	logger.Error("Failed to fetch TaskManager address", "err", err)
	// 	return nil, err
	// }
	// contractTaskManager, err := cstaskmanager.NewContractAlignedLayerTaskManager(taskManagerAddr, ethclient)
	// if err != nil {
	// 	logger.Error("Failed to fetch IAlignedLayerTaskManager contract", "err", err)
	// 	return nil, err
	// }

	return &AvsServiceBindings{
		ServiceManager: contractServiceManager,
		// TaskManager:    contractTaskManager,
		ethClient: ethclient,
		logger:    logger,
	}, nil
}
