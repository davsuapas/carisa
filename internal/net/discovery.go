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

package net

import (
	"log"
	"strconv"

	"github.com/carisa/internal/config"
	"github.com/hashicorp/consul/api"
	"go.uber.org/zap"
)

// HealthAddress returns the health address
func HealthAddress(srv config.Server, health config.Health) string {
	if len(srv.Address) == 0 || health.Port == 0 {
		log.Panic("The server and port for health service cannot be empty")
	}

	return srv.Address + ":" + strconv.Itoa(health.Port)
}

// Discovery is the general register service
type Discovery interface {
	// Register registers a service into discovery service
	Register(srv config.Server, health config.Health, name string)
	// DeRegister deregisters a service into discovery service
	Deregister(id string)
}

type ConsulDiscovery struct {
	log    *zap.Logger
	client *api.Client
}

// NewConsulDiscovery creates the consul discovery service
func NewConsulDiscovery(log *zap.Logger, srv string) *ConsulDiscovery {
	cConsul := api.DefaultConfig()
	if len(srv) > 0 {
		cConsul.Address = srv
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
func (d *ConsulDiscovery) Register(srv config.Server, health config.Health, name string) {
	d.log.Info("Registering server in consul ...", zap.String("ID", srv.ID))

	if len(name) == 0 {
		d.log.Panic(
			"The name cannot be empty",
			zap.String("ID", srv.ID),
			zap.String("Address", srv.Address))
	}
	if len(srv.Address) == 0 || srv.Port == 0 {
		d.log.Panic(
			"The address and port cannot be empty",
			zap.String("ID", srv.ID),
			zap.String("Address", srv.Address),
			zap.Int("Port", srv.Port))
	}

	check := &api.AgentServiceCheck{
		Name:                           srv.ID,
		Interval:                       strconv.Itoa(health.Interval) + "s",
		Timeout:                        strconv.Itoa(health.Timeout) + "s",
		TCP:                            HealthAddress(srv, health),
		FailuresBeforeCritical:         health.FailuresBeforeCritical,
		DeregisterCriticalServiceAfter: strconv.Itoa(health.Timeout) + "m",
	}
	sr := &api.AgentServiceRegistration{
		ID:      srv.ID,
		Name:    name,
		Tags:    []string{string(srv.NodeType)},
		Port:    srv.Port,
		Address: srv.Address,
		Check:   check,
	}
	if err := d.client.Agent().ServiceRegister(sr); err != nil {
		d.log.Panic(
			"The consul discovery client cannot register the agent",
			zap.String("Namespace", name),
			zap.String("ID", srv.ID),
			zap.String("Address", srv.Address),
			zap.Int("Port", srv.Port),
			zap.String("Error", err.Error()))
	}

	d.log.Info(
		"Service registered in consul",
		zap.String("ID", srv.ID),
		zap.String("Address", srv.Address),
		zap.Int("Port", srv.Port))
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
