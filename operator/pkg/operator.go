package operator

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"encoding/hex"
	"log"
	"time"

	"github.com/Layr-Labs/eigensdk-go/crypto/bls"
	"github.com/Layr-Labs/eigensdk-go/logging"
	eigentypes "github.com/Layr-Labs/eigensdk-go/types"
	"github.com/consensys/gnark-crypto/ecc"
	"github.com/consensys/gnark/backend/plonk"
	"github.com/consensys/gnark/backend/witness"
	ethcommon "github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/event"
	"github.com/yetanotherco/aligned_layer/common"
	servicemanager "github.com/yetanotherco/aligned_layer/contracts/bindings/AlignedLayerServiceManager"
	"github.com/yetanotherco/aligned_layer/core/chainio"
	"golang.org/x/crypto/sha3"

	"github.com/yetanotherco/aligned_layer/core/config"
)

type Operator struct {
	Config             config.OperatorConfig
	Address            ethcommon.Address
	Socket             string
	Timeout            time.Duration
	PrivKey            *ecdsa.PrivateKey
	KeyPair            *bls.KeyPair
	OperatorId         eigentypes.OperatorId
	avsSubscriber      chainio.AvsSubscriber
	NewTaskCreatedChan chan *servicemanager.ContractAlignedLayerServiceManagerNewTaskCreated
	Logger             logging.Logger
	//Socket  string
	//Timeout time.Duration
	//OperatorId         eigentypes.OperatorId
}

func NewOperatorFromConfig(configuration config.OperatorConfig) (*Operator, error) {
	logger := configuration.BaseConfig.Logger

	avsSubscriber, err := chainio.NewAvsSubscriberFromConfig(configuration.BaseConfig)
	if err != nil {
		log.Fatalf("Could not create AVS subscriber")
	}
	newTaskCreatedChan := make(chan *servicemanager.ContractAlignedLayerServiceManagerNewTaskCreated)

	address := configuration.Operator.Address
	operator := &Operator{
		Config:             configuration,
		Logger:             logger,
		avsSubscriber:      *avsSubscriber,
		Address:            address,
		NewTaskCreatedChan: newTaskCreatedChan,
		// Timeout
		// OperatorId
		// Socket
	}

	return operator, nil
}

func (o *Operator) SubscribeToNewTasks() event.Subscription {
	sub := o.avsSubscriber.SubscribeToNewTasks(o.NewTaskCreatedChan)
	return sub
}

func (o *Operator) Start(ctx context.Context) error {
	sub := o.SubscribeToNewTasks()
	for {
		select {
		case <-context.Background().Done():
			log.Println("Operator shutting down...")
			return nil
		case err := <-sub.Err():
			log.Println("Error in websocket subscription", "err", err)
			sub.Unsubscribe()
			sub = o.SubscribeToNewTasks()
		case newTaskCreatedLog := <-o.NewTaskCreatedChan:
			/* --------- OPERATOR MAIN LOGIC --------- */
			taskResponse := o.ProcessNewTaskCreatedLog(newTaskCreatedLog)
			// signedTaskResponse, err := o.SignTaskResponse(taskResponse)
			// if err != nil {
			// 	continue
			// }
			// go o.aggregatorRpcClient.SendSignedTaskResponseToAggregator(signedTaskResponse)

			/* --------- OPERATOR MAIN LOGIC --------- */
			log.Printf("The received task's index is: %d\n", newTaskCreatedLog.TaskIndex)

			// Here we should process a task, here we will pretend the proof is always true until adding that
			encodedResponseBytes, _ := AbiEncodeTaskResponse(taskResponse)
			log.Println("Task response:", taskResponse)
			log.Println("ABI Encoded bytes:\n", encodedResponseBytes)

			var taskResponseDigest [32]byte
			hasher := sha3.NewLegacyKeccak256()
			hasher.Write(encodedResponseBytes)
			copy(taskResponseDigest[:], hasher.Sum(nil)[:32])
			log.Println("Encoded response hash:", taskResponseDigest)
			log.Println("Encoded response hash len:", len(taskResponseDigest))
			responseSignature := *o.Config.BlsConfig.KeyPair.SignMessage(taskResponseDigest)
			log.Println("Signed hash:", responseSignature)
		}
	}
}

// Takes a NewTaskCreatedLog struct as input and returns a TaskResponseHeader struct.
// The TaskResponseHeader struct is the struct that is signed and sent to the contract as a task response.
func (o *Operator) ProcessNewTaskCreatedLog(newTaskCreatedLog *servicemanager.ContractAlignedLayerServiceManagerNewTaskCreated) *servicemanager.AlignedLayerServiceManagerTaskResponse {
	proof := newTaskCreatedLog.Task.Proof
	proofLen := (uint)(len(proof))

	pubInput := newTaskCreatedLog.Task.PubInput
	// pubInputLen := (uint)(len(pubInput))

	verifierId := newTaskCreatedLog.Task.ProvingSystemId

	o.Logger.Info("Received new task with proof to verify",
		"proof length", proofLen,
		"proof first bytes", "0x"+hex.EncodeToString(proof[0:8]),
		"proof last bytes", "0x"+hex.EncodeToString(proof[proofLen-8:proofLen]),
		"task index", newTaskCreatedLog.TaskIndex,
		"task created block", newTaskCreatedLog.Task.TaskCreatedBlock,
		// "quorumNumbers", newTaskCreatedLog.Task.QuorumNumbers,
		// "QuorumThresholdPercentage", newTaskCreatedLog.Task.QuorumThresholdPercentage,
	)

	switch verifierId {
	case uint16(common.GnarkPlonkBls12_381):
		verificationKey := newTaskCreatedLog.Task.VerificationKey
		VerificationResult := o.VerifyPlonkProof(proof, pubInput, verificationKey)

		o.Logger.Infof("PLONK proof verification result: %t", VerificationResult)
		taskResponse := &servicemanager.AlignedLayerServiceManagerTaskResponse{
			TaskIndex:      newTaskCreatedLog.TaskIndex,
			ProofIsCorrect: VerificationResult,
		}
		return taskResponse

	default:
		o.Logger.Error("Unrecognized verifier id")
		return nil
	}
}

func (o *Operator) VerifyPlonkProof(proofBytes []byte, pubInputBytes []byte, verificationKeyBytes []byte) bool {
	proofReader := bytes.NewReader(proofBytes)
	proof := plonk.NewProof(ecc.BLS12_381)
	_, err := proof.ReadFrom(proofReader)

	// If the proof can't be deserialized from the bytes then it doesn't verifies
	if err != nil {
		return false
	}

	pubInputReader := bytes.NewReader(pubInputBytes)
	pubInput, err := witness.New(ecc.BLS12_381.ScalarField())
	if err != nil {
		panic("Error instantiating witness")
	}
	_, err = pubInput.ReadFrom(pubInputReader)
	if err != nil {
		panic("Could not read PLONK public input")
	}
	verificationKeyReader := bytes.NewReader(verificationKeyBytes)
	verificationKey := plonk.NewVerifyingKey(ecc.BLS12_381)
	_, err = verificationKey.ReadFrom(verificationKeyReader)
	if err != nil {
		panic("Could not read PLONK verifying key from bytes")
	}

	err = plonk.Verify(proof, verificationKey, pubInput)

	return err == nil
}
