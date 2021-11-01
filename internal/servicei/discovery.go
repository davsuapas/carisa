/*
 *   Copyright (c) 2021 CARISA
 *   All rights reserved.

 *   Licensed under the Apache License, Version 2.0 (the "License");
 *   you may not use this file except in compliance with the License.
 *   You may obtain a copy of the License at

 *   http://www.apache.org/licenses/LICENSE-2.0

 *   Unless required by applicable law or agreed to in writing, software
 *   distributed under the License is distributed on an "AS IS" BASIS,
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *   See the License for the specific language governing permissions and
 *   limitations under the License.
 */

package servicei

import (
	"github.com/carisa/internal/config"
	"github.com/hashicorp/consul/api"
	"go.uber.org/zap"
)

// RegisterService is the register information
type RegisterService struct {
	ID        string
	Namespace string
	TypeNode  string
	Address   string
	Port      int
}

// Discovery is the general register service
type Discovery interface {
	Register(rs RegisterService)
	Deregister(id string)
}

type ConsulDiscovery struct {
	log    *zap.Logger
	client *api.Client
}

// NewConsulDiscovery creates the consul discovery service
func NewConsulDiscovery(log *zap.Logger, config config.Discovery) *ConsulDiscovery {
	cConsul := api.DefaultConfig()
	if len(config.Server) > 0 {
		cConsul.Address = config.Server
	}
	client, err := api.NewClient(cConsul)
	if err != nil {
		log.Panic("The consul discovery client cannot be created", zap.String("Error", err.Error()))
	}
	return &ConsulDiscovery{
		log:    log,
		client: client,
	}
}

// Register registers a service into consul
func (d *ConsulDiscovery) Register(rs RegisterService) {
	d.log.Info("Registering server in consul ...", zap.String("ID", rs.ID))

	if len(rs.Namespace) == 0 {
		d.log.Panic(
			"The Namespace cannot be empty",
			zap.String("ID", rs.ID),
			zap.String("Address", rs.Address))
	}
	if rs.Port == 0 {
		d.log.Panic(
			"The port cannot be zero",
			zap.String("ID", rs.ID),
			zap.String("Address", rs.Address))
	}
	sr := &api.AgentServiceRegistration{
		ID:      rs.ID,
		Name:    rs.Namespace,
		Tags:    []string{rs.TypeNode},
		Port:    rs.Port,
		Address: rs.Address,
	}
	if err := d.client.Agent().ServiceRegister(sr); err != nil {
		d.log.Panic("The consul discovery client cannot register the worker agent",
			zap.String("Namespace", rs.Namespace),
			zap.String("ID", rs.ID),
			zap.String("Address", rs.Address),
			zap.Int("Port", rs.Port),
			zap.String("Error", err.Error()))
	}

	d.log.Info("Service registered in consul", zap.String("ID", rs.ID))
}

// Deregister unregister the worker agent
func (d *ConsulDiscovery) Deregister(id string) {
	d.log.Info("Deregistering server in consul ...", zap.String("ID", id))

	if err := d.client.Agent().ServiceDeregister(id); err != nil {
		d.log.Panic("The consul discovery client cannot de-register the worker agent",
			zap.String("ID", id),
			zap.String("Error", err.Error()))
	}

	d.log.Info("Service de-registered in consul", zap.String("ID", id))
}

// ConvertTo converts from common config to RegisterService
func ConvertToRS(c config.Server, ns string) RegisterService {
	return RegisterService{
		ID:        c.ID,
		Namespace: ns,
		TypeNode:  c.TypeNode,
		Address:   c.Address,
		Port:      c.Port,
	}
}
