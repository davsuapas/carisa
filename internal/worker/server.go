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

package worker

import (
	"os"
	"os/signal"

	"github.com/carisa/internal/servicei"
	"go.uber.org/zap"
)

// Start starts the worker server
func Start(factory *Factory) {
	// Start server
	go func() {
		factory.log.Info(
			"Starting worker agent ...",
			zap.String("ID", factory.config.Server.ID),
			zap.String("Address", factory.config.Server.Address))

		factory.discovery.Register(
			servicei.ConvertToRS(factory.config.Server, factory.config.Namespace))

		factory.log.Info("Worker agent started")
	}()

	// Wait for interrupt signal to gracefully shutdown the server
	quit := make(chan os.Signal, 1)
	signal.Notify(quit, os.Interrupt)
	<-quit

	factory.log.Info(
		"Stopping worker agent ...",
		zap.String("ID", factory.config.Server.ID),
		zap.String("Address", factory.config.Server.Address))

	factory.discovery.Deregister(factory.config.Server.ID)

	factory.log.Info("Worker agent stopped")
}
