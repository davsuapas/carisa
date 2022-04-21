/*
 *   Copyright (c) 2022 CARISA
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
	"net"

	"go.uber.org/zap"
)

// Health define the interface for health service. This service listen a address
// that is called by discovery service to check the heatlh
type Health interface {
	// Run starts the heatlh service
	Run()
	// Stop stops the heatlh service
	Stop()
}

// NewTCPHealth creates a tcp health service
func NewTCPHealth(log *zap.Logger, srv string) Health {
	tcpAddr, err := net.ResolveTCPAddr("tcp", srv)
	if err != nil {
		log.Panic(
			"Health service cannot resolve tcp address",
			zap.String("Error", err.Error()))
	}
	tcpls, err := net.ListenTCP("tcp", tcpAddr)
	if err != nil {
		log.Panic(
			"Health service cannot listen tcp address",
			zap.String("Error", err.Error()))
	}
	return &TCPHealth{
		log:   log,
		tcpls: tcpls,
		quit:  make(chan struct{}),
	}
}

// TCPHealth is a tcp health service
type TCPHealth struct {
	log   *zap.Logger
	tcpls *net.TCPListener
	quit  chan struct{}
}

// Run accepts tcp connections for health service
func (h *TCPHealth) Run() {
	go func() {
		for {
			conn, err := h.tcpls.Accept()
			if err != nil {
				select {
				case <-h.quit:
					return
				default:
					h.log.Error(
						"Health service cannot accept tcp address",
						zap.String("Error", err.Error()))
				}
			}
			conn.Close()
		}
	}()
}

// Stop stops tcp connections for health service
func (h *TCPHealth) Stop() {
	close(h.quit)
	h.tcpls.Close()
}
